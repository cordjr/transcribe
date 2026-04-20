use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::channel::Sender;

use crate::media;

const TARGET_SAMPLE_RATE: u32 = 16_000;

// ============================================================================
// macOS: ScreenCaptureKit
// ============================================================================

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use screencapturekit::shareable_content::SCShareableContent;
    use screencapturekit::stream::content_filter::SCContentFilter;
    use screencapturekit::stream::output_trait::SCStreamOutputTrait;
    use screencapturekit::stream::output_type::SCStreamOutputType;
    use screencapturekit::stream::sc_stream::SCStream;
    use screencapturekit::stream::configuration::SCStreamConfiguration;
    use screencapturekit::cm::CMSampleBuffer;

    const CAPTURE_SAMPLE_RATE: i32 = 48000;
    const CAPTURE_CHANNELS: i32 = 2;

    struct AudioHandler {
        sender: Sender<Vec<f32>>,
        resample_ratio: f64,
        channels: usize,
    }

    impl SCStreamOutputTrait for AudioHandler {
        fn did_output_sample_buffer(
            &self,
            sample: CMSampleBuffer,
            of_type: SCStreamOutputType,
        ) {
            if of_type != SCStreamOutputType::Audio {
                return;
            }

            let Some(audio_list) = sample.audio_buffer_list() else {
                return;
            };

            for buf_ref in audio_list.iter() {
                let raw = buf_ref.data();
                if raw.is_empty() {
                    continue;
                }

                // Interpret as f32 PCM
                let f32_samples = unsafe {
                    std::slice::from_raw_parts(
                        raw.as_ptr() as *const f32,
                        raw.len() / std::mem::size_of::<f32>(),
                    )
                };

                let mono = media::to_mono(f32_samples, self.channels);
                let resampled = media::resample_linear(&mono, self.resample_ratio);

                if !resampled.is_empty() {
                    let _ = self.sender.send(resampled);
                }
            }
        }
    }

    pub fn capture(
        sender: Sender<Vec<f32>>,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let content = SCShareableContent::get()
            .map_err(|e| format!("Screen Recording permission required: {e}"))?;

        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| "No display found for audio capture".to_string())?;

        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();

        let config = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_sample_rate(CAPTURE_SAMPLE_RATE)
            .with_channel_count(CAPTURE_CHANNELS);

        let handler = AudioHandler {
            sender,
            resample_ratio: TARGET_SAMPLE_RATE as f64 / CAPTURE_SAMPLE_RATE as f64,
            channels: CAPTURE_CHANNELS as usize,
        };

        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(handler, SCStreamOutputType::Audio);

        stream.start_capture().map_err(|e| format!("Failed to start system audio capture: {e}"))?;

        while !stop_signal.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        stream.stop_capture().ok();
        Ok(())
    }
}

// ============================================================================
// Linux: cpal monitor source (PulseAudio/PipeWire)
// ============================================================================

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    pub fn capture(
        sender: Sender<Vec<f32>>,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| format!("Failed to list audio devices: {e}"))?;

        // Find a monitor source (PulseAudio/PipeWire exposes these as input devices)
        let mut monitor_device = None;
        for device in devices {
            if let Ok(name) = device.name() {
                if name.contains(".monitor") || name.contains("Monitor") {
                    monitor_device = Some(device);
                    break;
                }
            }
        }

        let device = monitor_device
            .ok_or_else(|| "No monitor source found. Ensure PulseAudio or PipeWire is running.".to_string())?;

        let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
        println!("System audio device: {device_name}");

        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get monitor config: {e}"))?;

        let source_rate = default_config.sample_rate().0;
        let source_channels = default_config.channels() as usize;
        let resample_ratio = TARGET_SAMPLE_RATE as f64 / source_rate as f64;

        let config = cpal::StreamConfig {
            channels: default_config.channels(),
            sample_rate: default_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono = media::to_mono(data, source_channels);
                    let resampled = if source_rate != TARGET_SAMPLE_RATE {
                        media::resample_linear(&mono, resample_ratio)
                    } else {
                        mono
                    };
                    if !resampled.is_empty() {
                        let _ = sender.send(resampled);
                    }
                },
                move |err| {
                    eprintln!("Monitor stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("Failed to build monitor stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start monitor stream: {e}"))?;

        while !stop_signal.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        drop(stream);
        Ok(())
    }
}

// ============================================================================
// Windows: WASAPI loopback
// ============================================================================

#[cfg(target_os = "windows")]
mod windows_audio {
    use super::*;

    pub fn capture(
        sender: Sender<Vec<f32>>,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<(), String> {
        use wasapi::*;

        // Initialize COM
        initialize_mta().map_err(|e| format!("COM init failed: {e}"))?;

        // Get default render device for loopback
        let device = get_default_device(&Direction::Render)
            .map_err(|e| format!("No render device: {e}"))?;

        let mut audio_client = device
            .get_iaudioclient()
            .map_err(|e| format!("Failed to get audio client: {e}"))?;

        let mix_format = audio_client
            .get_mixformat()
            .map_err(|e| format!("Failed to get mix format: {e}"))?;

        let source_rate = mix_format.get_samplespersec() as u32;
        let source_channels = mix_format.get_nchannels() as usize;
        let resample_ratio = TARGET_SAMPLE_RATE as f64 / source_rate as f64;

        let sharemode = ShareMode::Shared;
        let (_, _) = audio_client
            .get_periods()
            .map_err(|e| format!("Failed to get periods: {e}"))?;

        audio_client
            .initialize_client(
                &mix_format,
                0,
                &Direction::Capture,
                &sharemode,
                true, // loopback
            )
            .map_err(|e| format!("Failed to init loopback: {e}"))?;

        let capture_client = audio_client
            .get_audiocaptureclient()
            .map_err(|e| format!("Failed to get capture client: {e}"))?;

        let h_event = audio_client
            .set_get_eventhandle()
            .map_err(|e| format!("Failed to set event handle: {e}"))?;

        audio_client
            .start_stream()
            .map_err(|e| format!("Failed to start loopback: {e}"))?;

        println!("System audio: WASAPI loopback");

        while !stop_signal.load(Ordering::Relaxed) {
            if h_event.wait_for_event(200).is_err() {
                continue;
            }

            match capture_client.get_next_nbr_frames() {
                Ok(0) | Err(_) => continue,
                Ok(_) => {}
            }

            if let Ok(data) = capture_client.read_from_device_to_deque(
                mix_format.get_blockalign() as usize,
            ) {
                // Convert bytes to f32
                let f32_samples: Vec<f32> = data
                    .iter()
                    .collect::<Vec<_>>()
                    .chunks(4)
                    .filter_map(|chunk| {
                        if chunk.len() == 4 {
                            Some(f32::from_le_bytes([*chunk[0], *chunk[1], *chunk[2], *chunk[3]]))
                        } else {
                            None
                        }
                    })
                    .collect();

                let mono = media::to_mono(&f32_samples, source_channels);
                let resampled = if source_rate != TARGET_SAMPLE_RATE {
                    media::resample_linear(&mono, resample_ratio)
                } else {
                    mono
                };

                if !resampled.is_empty() {
                    let _ = sender.send(resampled);
                }
            }
        }

        audio_client.stop_stream().ok();
        Ok(())
    }
}

// ============================================================================
// Public API
// ============================================================================

pub fn capture_system_audio(
    sample_sender: Sender<Vec<f32>>,
    stop_signal: Arc<AtomicBool>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::capture(sample_sender, stop_signal)
    }

    #[cfg(target_os = "linux")]
    {
        linux::capture(sample_sender, stop_signal)
    }

    #[cfg(target_os = "windows")]
    {
        windows_audio::capture(sample_sender, stop_signal)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (sample_sender, stop_signal);
        Err("System audio capture not supported on this platform".to_string())
    }
}
