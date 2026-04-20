use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use std::error::Error;
use std::fs::{create_dir, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, fs, io, thread};

mod media;
mod system_audio;
mod vad;

use vad::VadChunker;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperError};

const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";

enum AppMode {
    FileTranscription {
        input_video: String,
        output_dir: String,
        language: String,
    },
    LiveCapture {
        output_file: String,
        audio_device: Option<String>,
        language: String,
        system_audio: bool,
    },
}

fn main() {
    let mode = match get_args() {
        Ok(m) => m,
        Err(error) => {
            println!("Error: {}", error);
            return;
        }
    };

    match mode {
        AppMode::FileTranscription {
            input_video,
            output_dir,
            language,
        } => run_file_mode(&input_video, &output_dir, &language),
        AppMode::LiveCapture {
            output_file,
            audio_device,
            language,
            system_audio,
        } => {
            if let Err(e) = run_live_mode(&output_file, audio_device.as_deref(), &language, system_audio) {
                println!("Error in live mode: {}", e);
            }
        }
    }
}

fn run_file_mode(input_video: &str, output_path: &str, language: &str) {
    let mut files_to_remove: Vec<String> = vec![];

    let output_dir = Path::new(output_path);
    if !output_dir.exists() {
        let creation_result = create_dir(output_dir);
        if let Err(error) = &creation_result {
            println!("Error: {}", error);
            return;
        }
    }

    let workdir = transcribe_work_path().unwrap();
    let paths = match media::process_to_segments(Path::new(input_video), workdir.as_path()) {
        Ok(v) => v,
        Err(e) => {
            println!("Error processing media: {}", e);
            return;
        }
    };
    for path in paths {
        transcribe(&path, output_path, language).unwrap();
        files_to_remove.push(path.clone());
    }
    println!("🧹 Cleaning temporary files...");
    files_to_remove.iter().for_each(|file| {
        let file_path = Path::new(file);
        if file_path.exists() {
            let _ = fs::remove_file(file_path);
        }
    });
    println!("✅ All good video file has been transcribed.!");
}

fn run_live_mode(
    output_file: &str,
    audio_device: Option<&str>,
    language: &str,
    enable_system_audio: bool,
) -> Result<(), Box<dyn Error>> {
    let model_path = model_full_path()?;
    let model_path_str = model_path.to_str().unwrap().to_string();
    download_model_if_missing(&model_path_str, WHISPER_MODEL_URL)?;

    let ctx = Arc::new(load_whisper_model(&model_path_str));
    let language = language.to_string();

    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_signal.clone();
    ctrlc::set_handler(move || {
        println!("\nStopping...");
        stop_clone.store(true, Ordering::Relaxed);
    })
    .expect("Error setting Ctrl+C handler");

    // Channels
    let (mic_tx, mic_rx) = crossbeam::channel::bounded::<Vec<f32>>(16);
    let (sys_tx, sys_rx) = crossbeam::channel::bounded::<Vec<f32>>(16);
    let (sample_tx, sample_rx) = crossbeam::channel::bounded::<Vec<f32>>(16);
    let (chunk_tx, chunk_rx) = crossbeam::channel::bounded::<Vec<f32>>(8);
    let (text_tx, text_rx) = crossbeam::channel::bounded::<String>(8);

    // Thread 1: Mic capture
    let device_owned = audio_device.map(|s| s.to_string());
    let stop_capture = stop_signal.clone();
    let capture_handle = thread::spawn(move || {
        let device_ref = device_owned.as_deref();
        if let Err(e) = media::capture_live_audio(device_ref, mic_tx, stop_capture) {
            eprintln!("Mic capture error: {e}");
        }
    });

    // Thread 2: System audio capture (optional)
    let sys_handle = if enable_system_audio {
        let sys_sender = sys_tx.clone();
        let stop_sys = stop_signal.clone();
        let handle = thread::spawn(move || {
            match system_audio::capture_system_audio(sys_sender, stop_sys) {
                Ok(()) => {}
                Err(e) => eprintln!("System audio not available: {e}"),
            }
        });
        drop(sys_tx);
        Some(handle)
    } else {
        drop(sys_tx);
        None
    };

    // Thread 3: Mixer
    let sys_rx_opt = if enable_system_audio {
        Some(sys_rx)
    } else {
        drop(sys_rx);
        None
    };
    let stop_mixer = stop_signal.clone();
    let mixer_handle = thread::spawn(move || {
        media::run_mixer(mic_rx, sys_rx_opt, sample_tx, stop_mixer);
    });

    // Thread 2: VAD chunking
    let stop_vad = stop_signal.clone();
    let vad_handle = thread::spawn(move || {
        let mut chunker = VadChunker::new();
        loop {
            match sample_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(samples) => {
                    if let Some(chunk) = chunker.feed(&samples) {
                        if chunk_tx.send(chunk).is_err() {
                            break;
                        }
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    if stop_vad.load(Ordering::Relaxed) {
                        if let Some(chunk) = chunker.flush() {
                            let _ = chunk_tx.send(chunk);
                        }
                        break;
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    if let Some(chunk) = chunker.flush() {
                        let _ = chunk_tx.send(chunk);
                    }
                    break;
                }
            }
        }
    });

    // Thread 4: Transcription
    let ctx_clone = ctx.clone();
    let lang_clone = language.clone();
    let transcribe_handle = thread::spawn(move || {
        while let Ok(chunk) = chunk_rx.recv() {
            match transcribe_samples(&ctx_clone, &chunk, &lang_clone) {
                Ok(text) if !text.is_empty() => {
                    if text_tx.send(text).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("Transcription error: {e}"),
            }
        }
    });

    // Main thread: write output
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_file)?;

    if enable_system_audio {
        println!("Capturing: microphone + system audio");
    } else {
        println!("Capturing: microphone only (use --system-audio to include call audio)");
    }
    println!("Listening... (press Ctrl+C to stop)");
    println!("Output file: {output_file}\n");

    while let Ok(text) = text_rx.recv() {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("[{timestamp}] {text}\n");
        print!("{line}");
        file.write_all(line.as_bytes())?;
        file.flush()?;
    }

    // Wait for threads to finish
    let _ = capture_handle.join();
    if let Some(h) = sys_handle {
        let _ = h.join();
    }
    let _ = mixer_handle.join();
    let _ = vad_handle.join();
    let _ = transcribe_handle.join();

    println!("\nDone. Transcription saved to: {output_file}");
    Ok(())
}

fn model_full_path() -> Result<PathBuf, Box<dyn Error>> {
    let model_path = dirs::home_dir().unwrap().join(Path::new(".whisper-model"));
    if !model_path.exists() {
        if let Err(error) = fs::create_dir(&model_path) {
            return Err(Box::new(error));
        }
    }
    Ok(model_path.join("ggml-small.bin"))
}
fn transcribe_work_path() -> Result<PathBuf, Box<dyn Error>> {
    let workdir_path = dirs::home_dir()
        .unwrap()
        .join(Path::new(".transcribe-workdir"));
    if !workdir_path.exists() {
        if let Err(error) = fs::create_dir(&workdir_path) {
            return Err(Box::new(error));
        }
    }
    Ok(workdir_path)
}

fn get_args() -> Result<AppMode, Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();

    let mut input_video = None;
    let mut output_dir = None;
    let mut output_file = None;
    let mut audio_device = None;
    let mut language: String = "pt".to_string();
    let mut live = false;
    let mut system_audio = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--live" => {
                live = true;
            }
            "--system-audio" => {
                system_audio = true;
            }
            "--input-video" => {
                input_video = args.get(i + 1).cloned();
                i += 1;
            }
            "--output-dir" => {
                output_dir = args.get(i + 1).cloned();
                i += 1;
            }
            "--output-file" => {
                output_file = args.get(i + 1).cloned();
                i += 1;
            }
            "--audio-device" => {
                audio_device = args.get(i + 1).cloned();
                i += 1;
            }
            "--language" => {
                if let Some(val) = args.get(i + 1) {
                    language = val.clone();
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if live {
        let output_file = output_file.ok_or("--output-file is required with --live")?;
        Ok(AppMode::LiveCapture {
            output_file,
            audio_device,
            language,
            system_audio,
        })
    } else {
        let input_video = input_video.ok_or("--input-video is required")?;
        let output_dir = output_dir.ok_or("--output-dir is required")?;
        if !Path::new(&input_video).exists() {
            Err(Box::from(format!(
                "❌ Input video does not exist: {}",
                input_video
            )))
        } else {
            Ok(AppMode::FileTranscription {
                input_video,
                output_dir,
                language,
            })
        }
    }
}

/// Suppress stderr output from C libraries (whisper model loading logs).
/// Returns a guard that restores stderr when dropped.
fn suppress_stderr() -> Option<i32> {
    unsafe {
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY);
        if devnull < 0 {
            return None;
        }
        let saved = libc::dup(libc::STDERR_FILENO);
        libc::dup2(devnull, libc::STDERR_FILENO);
        libc::close(devnull);
        Some(saved)
    }
}

fn restore_stderr(saved_fd: Option<i32>) {
    if let Some(fd) = saved_fd {
        unsafe {
            libc::dup2(fd, libc::STDERR_FILENO);
            libc::close(fd);
        }
    }
}

fn load_whisper_model(model_path_str: &str) -> WhisperContext {
    print!("Loading transcription model...");
    io::stdout().flush().ok();
    let saved = suppress_stderr();
    let ctx = WhisperContext::new(model_path_str).expect("Failed to load whisper model");
    restore_stderr(saved);
    println!(" done.");
    ctx
}

fn transcribe_samples(
    ctx: &WhisperContext,
    samples: &[f32],
    language: &str,
) -> Result<String, WhisperError> {
    let mut state = ctx.create_state().expect("Failed to create whisper state");
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(language));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);

    let saved = suppress_stderr();
    let result = state.full(params, samples);
    restore_stderr(saved);
    result?;

    let n_segments = state.full_n_segments()?;
    let mut result = String::new();
    for i in 0..n_segments {
        if let Ok(texto) = state.full_get_segment_text(i) {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(texto.trim());
        }
    }
    Ok(result)
}

fn transcribe(audio_path: &str, output_path: &str, language: &str) -> Result<(), WhisperError> {
    let model_path = match model_full_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let model_path_str = model_path.to_str().unwrap();

    let download_result = download_model_if_missing(model_path_str, WHISPER_MODEL_URL);
    if let Err(e) = download_result {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    let local_audio_path = Path::new(audio_path);
    if !local_audio_path.exists() {
        eprintln!("❌ Áudio não encontrado: {}", audio_path);
        std::process::exit(1);
    }

    let ctx = load_whisper_model(model_path_str);
    let mut reader = hound::WavReader::open(audio_path).expect("Failed to open audio file");
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    println!("⏳ transcribing...");
    let text = transcribe_samples(&ctx, &samples, language)?;
    println!("📝 Transcription finished:\n");

    let caminho_saida = format!(
        "{output_path}/transcricao_{}.txt",
        local_audio_path.file_name().unwrap().to_str().unwrap()
    );
    let mut arquivo_txt = File::create(&caminho_saida).unwrap_or_else(|_| {
        panic!("Erro ao criar arquivo de transcrição -> {}", caminho_saida)
    });
    writeln!(arquivo_txt, "{}", text).expect("❌ Error writing output file");
    Ok(())
}


fn clean_workdir_files() {
    let work_path_result = transcribe_work_path();
    if let Err(err) = &work_path_result {
        println!("{err}");
        return;
    }

    let binding = work_path_result.unwrap();

    let path = binding.as_path();
    let read_dir_result = fs::read_dir(path);
    if let Err(e) = read_dir_result {
        eprintln!("{e}");
        return;
    }
    for entry in fs::read_dir(path).unwrap() {
        let local_entry = entry.expect("❌");
        let local_path = local_entry.path();
        let file_name = local_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        if local_path.is_file() {
            match fs::canonicalize(local_path) {
                Ok(full_path) => {
                    let remove_file_result = fs::remove_file(full_path);
                    if let Err(e) = remove_file_result {
                        println!("Error removing file {}", e);
                    }
                }
                Err(_) => {
                    println!("❌ cleaning files: {}", file_name);
                }
            }
        }
    }
}
pub fn download_model_if_missing(model_path: &str, url: &str) -> io::Result<()> {
    if Path::new(model_path).exists() {
        return Ok(());
    }

    println!("Downloading transcription model (first run only)...");

    let client = Client::new();
    let response = client
        .get(url)
        .send()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let total_size = response.content_length().ok_or(io::Error::new(
        io::ErrorKind::Other,
        "❌ Tamanho de conteúdo desconhecido",
    ))?;

    let mut file = File::create(model_path)?;

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap(),
    );

    let mut downloaded = 0;
    let mut buffer = [0; 8192];
    let mut stream = response;

    while let Ok(n) = stream.read(&mut buffer) {
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("✅ Download finished");
    Ok(())
}
