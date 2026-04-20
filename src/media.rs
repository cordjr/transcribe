use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam::channel::{Receiver, Sender};
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

    graph.parse(&filter_spec).map_err(err_s)?;
    graph.validate().map_err(err_s)?;

    Ok(graph)
}

fn err_s<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}

// --- Live audio capture via cpal ---

const TARGET_SAMPLE_RATE: u32 = 16_000;

pub fn capture_live_audio(
    device_name: Option<&str>,
    sample_sender: Sender<Vec<f32>>,
    stop_signal: Arc<AtomicBool>,
) -> Result<(), String> {
    let host = cpal::default_host();

    let device = match device_name {
        Some(name) => {
            let devices = host.input_devices().map_err(|e| format!("Failed to list input devices: {e}"))?;
            let mut found = None;
            for d in devices {
                if let Ok(n) = d.name() {
                    if n == name {
                        found = Some(d);
                        break;
                    }
                }
            }
            found.ok_or_else(|| format!("Audio device '{name}' not found"))?
        }
        None => host
            .default_input_device()
            .ok_or_else(|| "No default input device available".to_string())?,
    };

    let device_display_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    let default_config = device
        .default_input_config()
        .map_err(|e| format!("Failed to get default input config: {e}"))?;

    let source_rate = default_config.sample_rate().0;
    let source_channels = default_config.channels() as usize;

    println!("Audio device: {device_display_name}");

    let config = cpal::StreamConfig {
        channels: default_config.channels(),
        sample_rate: default_config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let sender = sample_sender.clone();
    let resample_ratio = TARGET_SAMPLE_RATE as f64 / source_rate as f64;

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mono = to_mono(data, source_channels);

                // Resample to 16kHz using linear interpolation
                let resampled = if source_rate != TARGET_SAMPLE_RATE {
                    resample_linear(&mono, resample_ratio)
                } else {
                    mono
                };

                if !resampled.is_empty() {
                    let _ = sender.send(resampled);
                }
            },
            move |err| {
                eprintln!("Audio stream error: {err}");
            },
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {e}"))?;

    stream.play().map_err(|e| format!("Failed to start audio stream: {e}"))?;

    // Keep the stream alive until stop signal
    while !stop_signal.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    drop(stream);
    Ok(())
}

pub(crate) fn to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub(crate) fn resample_linear(input: &[f32], ratio: f64) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let output_len = (input.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let sample = if idx + 1 < input.len() {
            input[idx] * (1.0 - frac) + input[idx + 1] * frac
        } else if idx < input.len() {
            input[idx]
        } else {
            0.0
        };
        output.push(sample);
    }
    output
}

// --- Audio mixer ---

pub fn run_mixer(
    mic_rx: Receiver<Vec<f32>>,
    sys_rx: Option<Receiver<Vec<f32>>>,
    out_tx: Sender<Vec<f32>>,
    stop_signal: Arc<AtomicBool>,
) {
    let mut mic_buf: Vec<f32> = Vec::new();
    let mut sys_buf: Vec<f32> = Vec::new();
    let timeout = std::time::Duration::from_millis(50);

    loop {
        // Drain mic channel
        match mic_rx.recv_timeout(timeout) {
            Ok(samples) => mic_buf.extend_from_slice(&samples),
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                if stop_signal.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
        while let Ok(samples) = mic_rx.try_recv() {
            mic_buf.extend_from_slice(&samples);
        }

        // Drain system audio channel
        if let Some(ref sys) = sys_rx {
            while let Ok(samples) = sys.try_recv() {
                sys_buf.extend_from_slice(&samples);
            }
        }

        // Mix and send
        if !mic_buf.is_empty() || !sys_buf.is_empty() {
            let mixed = mix_buffers(&mut mic_buf, &mut sys_buf);
            if !mixed.is_empty() {
                if out_tx.send(mixed).is_err() {
                    break;
                }
            }
        }

        if stop_signal.load(Ordering::Relaxed)
            && mic_buf.is_empty()
            && sys_buf.is_empty()
        {
            break;
        }
    }
}

fn mix_buffers(mic: &mut Vec<f32>, sys: &mut Vec<f32>) -> Vec<f32> {
    if sys.is_empty() {
        return std::mem::take(mic);
    }
    if mic.is_empty() {
        return std::mem::take(sys);
    }

    let len = mic.len().max(sys.len());
    let mut mixed = Vec::with_capacity(len);
    for i in 0..len {
        let m = if i < mic.len() { mic[i] } else { 0.0 };
        let s = if i < sys.len() { sys[i] } else { 0.0 };
        mixed.push((m + s).clamp(-1.0, 1.0));
    }
    mic.clear();
    sys.clear();
    mixed
}
