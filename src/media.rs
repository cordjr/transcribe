use std::fs;
use std::path::{Path, PathBuf};

use ffmpeg::{codec, filter, format, frame, media};
use ffmpeg_next as ffmpeg;
use ffmpeg_next::filter::Graph;
use hound::{SampleFormat, WavSpec, WavWriter};

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1; // mono
const SEGMENT_SECONDS: u32 = 1800; // 30 minutes

// silenceremove parameters - less aggressive for better quality
const SILENCE_THRESHOLD_DB: i32 = -50; // dB (was -40, now less aggressive)
const SILENCE_WINDOW_SECS: f32 = 0.5; // seconds (was 0.3, now longer window)

pub fn process_to_segments(input_video: &Path, workdir: &Path) -> Result<Vec<String>, String> {
    // Ensure workdir exists
    if !workdir.exists() {
        fs::create_dir_all(workdir).map_err(|e| format!("workdir create error: {e}"))?;
    }

    ffmpeg::init().map_err(|e| format!("ffmpeg init error: {e}"))?;

    // Open container and find best audio stream
    let input_video_ref = PathBuf::from(input_video);
    let mut ictx = format::input(&input_video_ref).map_err(|e| format!("open input error: {e}"))?;
    let stream = ictx
        .streams()
        .best(media::Type::Audio)
        .ok_or_else(|| String::from("no audio stream found"))?;
    let stream_index = stream.index();

    let dec_ctx = codec::context::Context::from_parameters(stream.parameters()).map_err(err_s)?;
    let mut decoder = dec_ctx.decoder().audio().map_err(err_s)?;

    // Get decoder properties for filter graph input
    let in_sample_rate = decoder.rate();
    let in_sample_fmt = decoder.format().name();
    let in_channel_layout = if decoder.channel_layout().is_empty() {
        // Default to stereo if not specified
        "stereo".to_string()
    } else {
        format!("{:#x}", decoder.channel_layout().bits())
    };

    println!("📊 Input audio: {}Hz, format={}, channels={}",
             in_sample_rate, in_sample_fmt, in_channel_layout);

    // Create filter graph with high-quality resampling
    let mut graph = create_filter_graph(in_sample_rate, in_sample_fmt, &in_channel_layout)?;

    // Prepare segmentation writer state
    let seg_len_samples: usize = (SAMPLE_RATE as usize) * (SEGMENT_SECONDS as usize);
    let mut output_paths: Vec<String> = Vec::new();
    let mut current_writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>> = None;
    let mut written_in_segment: usize = 0;
    let mut part_idx: usize = 0;

    let open_segment = |part_idx: usize,
                        workdir: &Path,
                        paths: &mut Vec<String>|
     -> Result<WavWriter<std::io::BufWriter<std::fs::File>>, String> {
        let file_path = workdir.join(format!("part_{:03}.wav", part_idx));
        let spec = WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let writer = WavWriter::create(&file_path, spec)
            .map_err(|e| format!("wav create error: {e}"))?;
        let path_str = file_path.to_string_lossy().to_string();
        println!("📂 Opening segment file: {}", path_str);
        paths.push(path_str);
        Ok(writer)
    };

    // Process packets
    for (s, packet) in ictx.packets() {
        if s.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet).map_err(err_s)?;
        let mut frm = frame::Audio::empty();
        while decoder.receive_frame(&mut frm).is_ok() {
            // Feed decoded frame directly into filter graph (no manual resampling)
            {
                let mut in_filter = graph
                    .get("Parsed_abuffer_0")
                    .ok_or_else(|| "abuffer not found".to_string())?;
                let mut src = in_filter.source();
                src.add(&frm).map_err(err_s)?;
            }

            // Pull all available filtered frames
            process_filtered_frames(
                &mut graph,
                &mut current_writer,
                &mut written_in_segment,
                &mut part_idx,
                seg_len_samples,
                workdir,
                &mut output_paths,
                &open_segment,
            )?;
        }
    }
    decoder.send_eof().ok();

    // Flush the filter graph
    {
        if let Some(mut in_filter) = graph.get("Parsed_abuffer_0") {
            in_filter.source().flush().ok();
        }
    }

    // Drain remaining filtered frames
    process_filtered_frames(
        &mut graph,
        &mut current_writer,
        &mut written_in_segment,
        &mut part_idx,
        seg_len_samples,
        workdir,
        &mut output_paths,
        &open_segment,
    )?;

    // Close the last writer if open
    if let Some(w) = current_writer.take() {
        w.finalize().map_err(|e| format!("wav finalize: {e}"))?;
    }

    Ok(output_paths)
}

fn process_filtered_frames<F>(
    graph: &mut Graph,
    current_writer: &mut Option<WavWriter<std::io::BufWriter<std::fs::File>>>,
    written_in_segment: &mut usize,
    part_idx: &mut usize,
    seg_len_samples: usize,
    workdir: &Path,
    output_paths: &mut Vec<String>,
    open_segment: &F,
) -> Result<(), String>
where
    F: Fn(usize, &Path, &mut Vec<String>) -> Result<WavWriter<std::io::BufWriter<std::fs::File>>, String>,
{
    let mut filtered = frame::Audio::empty();
    loop {
        let mut out_filter = graph
            .get("Parsed_abuffersink_4")
            .ok_or_else(|| "abuffersink not found".to_string())?;
        let mut sink = out_filter.sink();
        if sink.frame(&mut filtered).is_err() {
            break;
        }

        // Write filtered samples to current segment, rotating by duration
        let data = filtered.data(0);
        let samples = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const i16, data.len() / 2)
        };

        for &sample in samples {
            if current_writer.is_none() {
                *current_writer = Some(open_segment(*part_idx, workdir, output_paths)?);
                *written_in_segment = 0;
            }
            if *written_in_segment >= seg_len_samples {
                if let Some(w) = current_writer.take() {
                    w.finalize().map_err(|e| format!("wav finalize: {e}"))?;
                }
                *part_idx += 1;
                *current_writer = Some(open_segment(*part_idx, workdir, output_paths)?);
                *written_in_segment = 0;
            }
            if let Some(w) = current_writer.as_mut() {
                w.write_sample(sample)
                    .map_err(|e| format!("wav write: {e}"))?;
            }
            *written_in_segment += 1;
        }
    }
    Ok(())
}

fn create_filter_graph(in_sample_rate: u32, in_sample_fmt: &str, in_channel_layout: &str) -> Result<Graph, String> {
    let mut graph = filter::Graph::new();

    // Input buffer with original audio format
    let abuffer_args = format!(
        "time_base=1/{rate}:sample_rate={rate}:sample_fmt={fmt}:channel_layout={ch}",
        rate = in_sample_rate,
        fmt = in_sample_fmt,
        ch = in_channel_layout
    );

    // Build filter chain:
    // 1. abuffer: input with original format
    // 2. aresample: high-quality resampling to 16kHz mono
    // 3. silenceremove: remove silence (less aggressive settings)
    // 4. aformat: ensure output is s16 mono
    // 5. abuffersink: output
    // Filter names will be auto-generated as Parsed_<filter>_<index>
    let filter_spec = format!(
        "abuffer={abuffer_args} [step1]; \
         [step1] aresample=out_sample_rate={out_rate}:out_chlayout=mono:filter_size=64:cutoff=0.97 [step2]; \
         [step2] silenceremove=start_periods=1:start_duration={win}:start_threshold={thr}dB:stop_periods=-1:stop_duration={win}:stop_threshold={thr}dB [step3]; \
         [step3] aformat=sample_fmts=s16:channel_layouts=mono:sample_rates={out_rate} [step4]; \
         [step4] abuffersink",
        abuffer_args = abuffer_args,
        out_rate = SAMPLE_RATE,
        win = SILENCE_WINDOW_SECS,
        thr = SILENCE_THRESHOLD_DB
    );

    println!("🔧 Filter spec: {}", filter_spec);

    graph.parse(&filter_spec).map_err(err_s)?;
    graph.validate().map_err(err_s)?;

    Ok(graph)
}

fn err_s<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}
