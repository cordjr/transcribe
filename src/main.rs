use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use std::error::Error;
use std::fs::{create_dir, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperError};
const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
fn main() {
    clean_workdir_files();
    let mut files_to_remove: Vec<String> = vec![];
    let get_args_result = get_args();
    if let Err(error) = &get_args_result {
        println!("Error: {}", error);
        return;
    }
    let (input_video, output_path, language) = get_args_result.unwrap();

    if !ffmpeg_installed() {
        println!("❌ FFmpeg is not installed or not in PATH.");
        return;
    }

    let output_dir = Path::new(&output_path);
    if !output_dir.exists() {
        let creation_result = create_dir(&output_dir);
        if let Err(error) = &creation_result {
            println!("Error: {}", error);
            return;
        }
    }

    let audio_path = extract_audio(&input_video).unwrap();
    files_to_remove.push(audio_path.clone());
    let clean_audion_path = clean_silence(&audio_path).unwrap();
    files_to_remove.push(clean_audion_path.clone());
    let paths = split_audio(&clean_audion_path).unwrap();
    for path in paths {
        transcribe(&path, &output_path, &language).unwrap();
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

fn get_args() -> Result<(String, String, String), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();

    let mut input_video = None;
    let mut output_dir = None;
    let mut language: String = "pt".to_string(); // default language
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--input-video" => {
                input_video = args.get(i + 1).cloned();
                i += 1;
            }
            "--output-dir" => {
                output_dir = args.get(i + 1).cloned();
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
    let input_video = input_video.expect("--input-video is required");
    let output_dir = output_dir.expect("--output-dir is required");
    if !Path::new(&input_video).exists() {
        Err(Box::from(format!(
            "❌ Input video does not exist: {}",
            input_video
        )))
    } else {
        Ok((input_video, output_dir, language))
    }
}

fn ffmpeg_installed() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn extract_audio(video_path: &str) -> Result<String, String> {
    let work_path = transcribe_work_path().unwrap();

    // Use file stem safely instead of assuming a specific extension
    let stem = Path::new(video_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid video filename: {}", video_path))?;

    // Always write the extracted WAV into the workdir
    let audio_raw_path = work_path.join(format!("{stem}.wav"));
    let audio_raw = audio_raw_path.to_str().unwrap().to_string();

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video_path,
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            &audio_raw,
        ])
        .status();
    match status {
        Ok(s) if s.success() => Ok(audio_raw),
        Ok(s) => Err(format!("ffmpeg failed with status: {}", s)),
        Err(e) => Err(format!("ffmpeg exited with error: {}", e)),
    }
}

fn clean_silence(audio_path: &str) -> Result<String, String> {
    let file_name = audio_path
        .split("/")
        .last()
        .unwrap()
        .to_string()
        .strip_suffix(".wav")
        .unwrap()
        .to_string();
    let binding = transcribe_work_path().unwrap();

    let work_path = binding.as_path();

    let audio_clean_path = work_path
        .join(format!("{}.wav", file_name))
        .to_str()
        .unwrap()
        .to_string();
    let result = Command::new("ffmpeg")
        .args([
            "-y", "-i", &audio_path,
            "-af", "silenceremove=start_periods=1:start_duration=0.3:start_threshold=-40dB:stop_periods=-1:stop_duration=0.3:stop_threshold=-40dB",
            &audio_clean_path,
        ])
        .status();
    match result {
        Ok(_) => Ok(audio_clean_path),
        Err(e) => Err(format!("ffmpeg exited with error: {}", e)),
    }
}

fn split_audio(audio_clean: &str) -> Result<Vec<String>, String> {
    let work_path = transcribe_work_path().unwrap();
    let output_file_pattern = work_path
        .join("parte_%03d.wav")
        .to_str()
        .unwrap()
        .to_string();

    let result = Command::new("ffmpeg")
        .args([
            "-i",
            &audio_clean,
            "-f",
            "segment",
            "-segment_time",
            "1800",
            "-c",
            "copy",
            &output_file_pattern,
        ])
        .status();
    match result {
        Ok(_) => list_files(work_path.as_path()),
        Err(e) => Err(format!("❌ ffmpeg exited with error: {}", e)),
    }
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

    let ctx = WhisperContext::new(model_path_str).expect("❌ Error loading model");
    // 🎧 Decodifica o áudio WAV para Vec<f32>
    let mut reader = hound::WavReader::open(audio_path).expect("❌ Error opening audio file");
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    let mut state = ctx.create_state().expect("❌ Error creating state");
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    params.set_language(Some(language));
    params.set_translate(false);

    println!("⏳ transcribing...");

    state.full(params, &samples).expect("Transcription Error");

    let num_segments_result = state.full_n_segments();
    println!("📝 Transcription finished:\n");
    match num_segments_result {
        Ok(n_segments) => {
            let caminho_saida = format!(
                "{output_path}/transcricao_{}.txt",
                local_audio_path.file_name().unwrap().to_str().unwrap()
            );
            let mut arquivo_txt = File::create(&caminho_saida).expect(&format!(
                "Erro ao criar arquivo de transcrição -> {}",
                caminho_saida
            ));
            for i in 0..n_segments {
                if let Ok(texto) = state.full_get_segment_text(i) {
                    writeln!(arquivo_txt, "{}", texto.trim())
                        .expect("❌ Error writing output file");
                }
            }
            Ok(())
        }
        Err(e) => {
            println!("❌Erro ao carregar o WAV");
            Err(e)
        }
    }
}

fn list_files(output_path: &Path) -> Result<Vec<String>, String> {
    let mut paths: Vec<String> = vec![];

    for entry in fs::read_dir(output_path).expect("❌ Error loading WAV file") {
        let local_entry = entry.expect("❌ Error loading WAV file");
        let local_path = local_entry.path();
        let file_name = local_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        if local_path.is_file() && file_name.contains("parte_") {
            let full_name = output_path.join(&file_name).to_str().unwrap().to_string();
            paths.push(full_name);
        }
    }
    Ok(paths)
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
        println!("✅ Whisper model present");
        return Ok(());
    }

    println!("⬇️  Downloading whisper model :\n{}", url);

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
