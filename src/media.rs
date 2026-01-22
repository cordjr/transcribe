use std::fs;
use std::path::{Path, PathBuf};

use ffmpeg::{
    codec, filter, format, frame, media, software::resampling::Context as Resampler, ChannelLayout,
};
use ffmpeg_next as ffmpeg;
use ffmpeg_next::filter::Graph;
use hound::{SampleFormat, WavSpec, WavWriter};

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1; // mono
const SEGMENT_SECONDS: u32 = 1800; // 30 minutes

// silenceremove parameters to match previous CLI usage exactly
const SILENCE_THRESHOLD_DB: i32 = -40; // dB
const SILENCE_WINDOW_SECS: f32 = 0.3;

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

    // Resample to 16kHz mono s16 packed (good for WAV + later processing)
    let mut resampler = Resampler::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        format::Sample::I16(format::sample::Type::Packed),
        ChannelLayout::MONO,
        SAMPLE_RATE,
    )
    .map_err(err_s)?;

    // Prepare filter graph: abuffer -> silenceremove -> abuffersink

    let mut graph = create_grapth()?;

    // Prepare segmentation writer state
    let seg_len_samples: usize = (SAMPLE_RATE as usize) * (SEGMENT_SECONDS as usize);
    let mut output_paths: Vec<String> = Vec::new();
    let mut current_writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>> = None;
    let mut written_in_segment: usize = 0;
    let mut part_idx: usize = 0;

    let open_next_writer = |idx: usize,
                            workdir: &Path|
     -> Result<WavWriter<std::io::BufWriter<std::fs::File>>, String> {
        let file_path = workdir.join(format!("part_{:03}.wav", idx));
        let spec = WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        WavWriter::create(&file_path, spec).map_err(|e| format!("wav create error: {e}"))
    };

    let open_segment = |part_idx: usize,
                            workdir: &Path,
                            paths: &mut Vec<String>|
     -> Result<WavWriter<std::io::BufWriter<std::fs::File>>, String> {
        let file_path = workdir.join(format!("part_{:03}.wav", part_idx));
        let writer = open_next_writer(part_idx, workdir)?;
        let path_str = file_path.to_string_lossy().to_string();
        println!("📂 Opening segment file: {}", path_str);
        paths.push(path_str);
        Ok(writer)
    };
    for (s, packet) in ictx.packets() {
        if s.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet).map_err(err_s)?;
        let mut frm = frame::Audio::empty();
        while decoder.receive_frame(&mut frm).is_ok() {
            let mut dst = frame::Audio::empty();
            resampler.run(&frm, &mut dst).map_err(err_s)?;
            // Feed resampled frame into filter graph
            {
                let mut in_filter = graph
                    .get("Parsed_abuffer_0")
                    .ok_or_else(|| "abuffer not found".to_string())?;
                let mut src = in_filter.source();
                src.add(&dst).map_err(err_s)?;
            }

            // Pull all available filtered frames
            let mut filtered = frame::Audio::empty();
            loop {
                let mut out_filter = graph
                    .get("Parsed_abuffersink_2")
                    .ok_or_else(|| "abuffersink not found".to_string())?;
                let mut sink = out_filter.sink();
                if sink.frame(&mut filtered).is_err() {
                    break;
                }
                // Write filtered samples to current segment, rotating by duration
                let data = filtered.data(0);
                let samples = data
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]));
                let mut count = 0;
                for sample in samples {
                    count += 1;
                    if current_writer.is_none() {
                        current_writer = Some(open_segment(part_idx, workdir, &mut output_paths)?);
                        written_in_segment = 0;
                    }
                    if written_in_segment >= seg_len_samples {
                        if let Some(w) = current_writer.take() {
                            w.finalize().map_err(|e| format!("wav finalize: {e}"))?;
                        }
                        part_idx += 1;
                        current_writer = Some(open_segment(part_idx, workdir, &mut output_paths)?);
                        written_in_segment = 0;
                    }
                    if let Some(w) = current_writer.as_mut() {
                        w.write_sample(sample)
                            .map_err(|e| format!("wav write: {e}"))?;
                    }
                    written_in_segment += 1;
                }
                if count > 0 {
                    println!("📝 Processed {} samples from filtered frame", count);
                }
            }
        }
    }
    decoder.send_eof().ok();

    // After decoder EOF, also drain any remaining from filter sink
    let mut filtered = frame::Audio::empty();
    loop {
        let mut out_filter = graph
            .get("Parsed_abuffersink_2")
            .ok_or_else(|| "abuffersink not found".to_string())?;
        let mut sink = out_filter.sink();
        if sink.frame(&mut filtered).is_err() {
            break;
        }
        let data = filtered.data(0);
        let samples = data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]));
        for sample in samples {
            if current_writer.is_none() {
                current_writer = Some(open_segment(part_idx, workdir, &mut output_paths)?);
                written_in_segment = 0;
            }
            if written_in_segment >= seg_len_samples {
                if let Some(w) = current_writer.take() {
                    w.finalize().map_err(|e| format!("wav finalize: {e}"))?;
                }
                part_idx += 1;
                current_writer = Some(open_segment(part_idx, workdir, &mut output_paths)?);
                written_in_segment = 0;
            }
            if let Some(w) = current_writer.as_mut() {
                w.write_sample(sample)
                    .map_err(|e| format!("wav write: {e}"))?;
            }
            written_in_segment += 1;
        }
    }

    // Close the last writer if open
    if let Some(w) = current_writer.take() {
        w.finalize().map_err(|e| format!("wav finalize: {e}"))?;
    }

    Ok(output_paths)
}

fn create_grapth() -> Result<Graph, String> {
    let mut graph = filter::Graph::new();
    let abuffer_args = format!(
        "time_base=1/{rate}:sample_rate={rate}:sample_fmt=s16:channel_layout=mono",
        rate = SAMPLE_RATE
    );

    // Build silenceremove filter spec
    // Note: we use names that we will also use when adding filters manually to the graph
    let filter_spec = format!(
        "abuffer={abuffer_args} [in]; [in] silenceremove=start_periods=1:start_duration={win}:start_threshold={thr}dB:stop_periods=-1:stop_duration={win}:stop_threshold={thr}dB [out]; [out] abuffersink",
        abuffer_args = abuffer_args,
        win = SILENCE_WINDOW_SECS,
        thr = SILENCE_THRESHOLD_DB
    );

    graph.parse(&filter_spec).map_err(err_s)?;
    graph.validate().map_err(err_s)?;
    Ok(graph)
}

fn err_s<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}
