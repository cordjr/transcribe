# 🎬 Video Transcriber

Hey there! This is a handy Rust tool that takes your video files and turns them into text transcriptions. Perfect for when you need to convert lectures, meetings, or any video content into readable text.

**Why Rust?** This baby runs about 4x faster than the same thing written in Python. Because who wants to wait around for transcriptions, right?

## What does it do?

This little program is like having a personal transcriptionist in your terminal. Here's the magic it performs:

1. **Takes your video file** (MP4, MOV, and most FFmpeg-supported formats) and extracts the audio from it
2. **Cleans up the audio** by removing those awkward silences (you know, the ones that make files unnecessarily long)
3. **Chops up long videos** into manageable 30-minute chunks (because nobody wants to process a 3-hour file in one go)
4. **Transcribes the audio** using OpenAI's Whisper model — default language is Portuguese, but you can pick another with `--language <code>` (e.g., `en`, `pt`, `es`)
5. **Saves nice text files** with all the transcribed content

The best part? It cleans up after itself, removing all those temporary files it created during the process. No mess left behind! 🧹

## Prerequisites

Before you dive in, you'll need:

- **Rust** installed on your machine (obviously!)
- **FFmpeg development libraries** (used via Rust bindings, no CLI calls). Install system packages so that `pkg-config` can find `libav*`:
  - On macOS: `brew install ffmpeg pkg-config`
  - On Ubuntu/Debian: `sudo apt install ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswresample-dev pkg-config`
  - On other Linux distros: install the equivalent `libav*` dev packages and `pkg-config`
  - Note: The FFmpeg CLI binary is NOT required anymore.

## Installation

### Option 1: Download Pre-built Binaries (Recommended)

Head over to the [Releases](https://github.com/cordjr/transcribe/releases) page and download the binary for your system:

| Platform | File | Notes |
|----------|------|-------|
| Linux x86_64 | `transcribe-linux-x86_64` | Most common Linux distros |
| macOS Intel | `transcribe-macos-x86_64` | Intel Macs |
| macOS Apple Silicon | `transcribe-macos-aarch64` | M1/M2/M3 Macs |
| Windows x86_64 | `transcribe-windows-x86_64.exe` | Windows 10/11 |

#### Linux

```bash
# Download and make executable
curl -LO https://github.com/cordjr/transcribe/releases/latest/download/transcribe-linux-x86_64
chmod +x transcribe-linux-x86_64

# Move to a directory in your PATH (optional)
sudo mv transcribe-linux-x86_64 /usr/local/bin/transcribe

# Run it
transcribe --input-video video.mp4 --output-dir output
```

#### macOS

```bash
# For Apple Silicon (M1/M2/M3)
curl -LO https://github.com/cordjr/transcribe/releases/latest/download/transcribe-macos-aarch64
chmod +x transcribe-macos-aarch64

# For Intel Macs
curl -LO https://github.com/cordjr/transcribe/releases/latest/download/transcribe-macos-x86_64
chmod +x transcribe-macos-x86_64

# Remove quarantine attribute (macOS security)
xattr -d com.apple.quarantine transcribe-macos-*

# Move to a directory in your PATH (optional)
sudo mv transcribe-macos-aarch64 /usr/local/bin/transcribe

# Run it
transcribe --input-video video.mp4 --output-dir output
```

#### Windows

1. Download `transcribe-windows-x86_64.exe` from the [Releases](https://github.com/cordjr/transcribe/releases) page
2. Open Command Prompt or PowerShell
3. Navigate to the download folder
4. Run:

```powershell
.\transcribe-windows-x86_64.exe --input-video video.mp4 --output-dir output
```

**Tip:** Add the folder containing the executable to your PATH for easier access.

### Option 2: Build from Source

Clone this bad boy and build it:

```bash
git clone https://github.com/cordjr/transcribe.git
cd transcribe
cargo build --release
```

## How to use it

It's dead simple! Just run:

```bash
cargo run -- --input-video your-video.mov --output-dir transcriptions --language en
```

Or if you built the release version:

```bash
./target/release/transcribe --input-video your-video.mp4 --output-dir transcriptions --language pt
```

### Real example

Say you've got a lecture recording called `physics_101.mov` and you want English:

```bash
cargo run -- --input-video physics_101.mov --output-dir physics_notes --language en
```

This will:
- Create a `physics_notes` directory (if it doesn't exist)
- Extract and process the audio
- Generate transcription files like `transcricao_parte_000.wav.txt`, `transcricao_parte_001.wav.txt`, etc.

## First time running?

Don't panic if the first run takes a while! The program needs to download the Whisper model (about 500MB) on its first run. It'll save it in `~/.whisper-model/` so future runs will be much faster.

You'll see a nice progress bar while it downloads:

```
⬇️  Downloading whisper model :
https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
[00:00:45] [████████████████████████████████████████] 500MB/500MB (00:00:00)
✅ Download finished
```

## What you'll get

After the program finishes (grab a coffee, it might take a while for long videos), you'll find:

- Text files in your output directory
- Each file contains the transcription for a ~30-minute segment
- Files are named sequentially: `transcricao_parte_000.wav.txt`, `transcricao_parte_001.wav.txt`, etc.

## Good to know

- **Video format**: Supports MP4, MOV, and most formats FFmpeg can read
- **Language**: Default is Portuguese (`pt`). Choose another with `--language <code>` (e.g., `en`, `pt`, `es`)
- **Temp files**: The program uses `~/.transcribe-workdir/` for temporary files but cleans everything up when done
- **Audio quality**: Better audio = better transcriptions. The program does its best to clean up the audio, but garbage in, garbage out!
- **Processing time**: Transcription takes time. A 1-hour video might take 10-15 minutes depending on your machine

### What changed recently
- The app now uses FFmpeg libraries directly (via `ffmpeg-next`) instead of spawning the `ffmpeg` CLI.
- Silence removal uses FFmpeg's `silenceremove` filter through libavfilter for parity with previous behavior.
- Chunk files are written as `part_000.wav`, `part_001.wav`, ... into `~/.transcribe-workdir/`.

## Troubleshooting

**"FFmpeg is not installed or not in PATH"**
- Make sure FFmpeg is installed and accessible from your terminal. Try running `ffmpeg -version` to check.

**"Input video does not exist"**
- Double-check your file path. The program needs the full path or relative path from where you're running it.

**Out of memory**
- Very long videos might cause issues. The program splits them into chunks, but if you're still having problems, try splitting your video first.

## Contributing

Found a bug? Want to add a feature? PRs are welcome! Just remember to:
- Run `cargo fmt` before committing
- Check `cargo clippy` for any warnings
- Keep the informal, friendly tone in docs 😊

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

Happy transcribing! 🎉

---

## 🇧🇷 Versão em Português

### 🎬 Transcritor de Vídeos

E aí! Essa é uma ferramenta bacana em Rust que pega seus arquivos de vídeo e transforma em transcrições de texto. Perfeito pra quando você precisa converter aulas, reuniões ou qualquer conteúdo em vídeo pra texto.

**Por que Rust?** Esse negócio roda umas 4x mais rápido que a mesma coisa escrita em Python. Porque ninguém quer ficar esperando as transcrições, né?

### O que ela faz?

Esse programinha é tipo ter um transcritor pessoal no seu terminal. Olha a mágica que ele faz:

1. **Pega seu arquivo de vídeo** (MP4, MOV e outros formatos suportados pelo FFmpeg) e extrai o áudio
2. **Limpa o áudio** removendo aqueles silêncios chatos (sabe aqueles que deixam os arquivos desnecessariamente longos?)
3. **Corta vídeos longos** em pedaços de 30 minutos (porque ninguém quer processar um arquivo de 3 horas de uma vez)
4. **Transcreve o áudio** usando o modelo Whisper da OpenAI — por padrão em português, mas você pode escolher com `--language <código>` (ex.: `en`, `pt`, `es`)
5. **Salva arquivos de texto** com todo o conteúdo transcrito

A melhor parte? Ele limpa toda a bagunça depois, removendo todos os arquivos temporários que criou durante o processo. Não deixa sujeira pra trás! 🧹

### Instalacao

#### Opcao 1: Baixar Binarios Pre-compilados (Recomendado)

Va ate a pagina de [Releases](https://github.com/cordjr/transcribe/releases) e baixe o binario pro seu sistema:

| Plataforma | Arquivo | Notas |
|------------|---------|-------|
| Linux x86_64 | `transcribe-linux-x86_64` | Maioria das distros Linux |
| macOS Intel | `transcribe-macos-x86_64` | Macs com Intel |
| macOS Apple Silicon | `transcribe-macos-aarch64` | Macs M1/M2/M3 |
| Windows x86_64 | `transcribe-windows-x86_64.exe` | Windows 10/11 |

**Linux:**
```bash
curl -LO https://github.com/cordjr/transcribe/releases/latest/download/transcribe-linux-x86_64
chmod +x transcribe-linux-x86_64
./transcribe-linux-x86_64 --input-video video.mp4 --output-dir saida
```

**macOS:**
```bash
# Para Apple Silicon (M1/M2/M3)
curl -LO https://github.com/cordjr/transcribe/releases/latest/download/transcribe-macos-aarch64
chmod +x transcribe-macos-aarch64
xattr -d com.apple.quarantine transcribe-macos-aarch64
./transcribe-macos-aarch64 --input-video video.mp4 --output-dir saida
```

**Windows:**
1. Baixe `transcribe-windows-x86_64.exe` da pagina de [Releases](https://github.com/cordjr/transcribe/releases)
2. Abra o Prompt de Comando ou PowerShell
3. Execute:
```powershell
.\transcribe-windows-x86_64.exe --input-video video.mp4 --output-dir saida
```

#### Opcao 2: Compilar do Codigo Fonte

```bash
git clone https://github.com/cordjr/transcribe.git
cd transcribe
cargo build --release
```

### Pre-requisitos (apenas para compilar do codigo fonte)

- **Rust** instalado na sua maquina
- **FFmpeg development libraries**:
  - No macOS: `brew install ffmpeg pkg-config`
  - No Ubuntu/Debian: `sudo apt install ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswresample-dev pkg-config`

### Como usar

É moleza! Só rodar:

```bash
cargo run -- --input-video seu-video.mov --output-dir transcricoes --language en
```

Ou se você compilou a versão release:

```bash
./target/release/transcribe --input-video seu-video.mp4 --output-dir transcricoes --language pt
```

### Exemplo real

Digamos que você tem uma gravação de aula chamada `fisica_101.mov` e quer em inglês:

```bash
cargo run -- --input-video fisica_101.mov --output-dir anotacoes_fisica --language en
```

Isso vai:
- Criar um diretório `anotacoes_fisica` (se não existir)
- Extrair e processar o áudio
- Gerar arquivos de transcrição tipo `transcricao_parte_000.wav.txt`, `transcricao_parte_001.wav.txt`, etc.

### Primeira vez rodando?

Não se assuste se a primeira execução demorar! O programa precisa baixar o modelo Whisper (uns 500MB) na primeira vez. Ele vai salvar em `~/.whisper-model/` então as próximas execuções serão bem mais rápidas.

### Bom saber

- **Formato de vídeo**: Suporta MP4, MOV e a maioria dos formatos que o FFmpeg consegue abrir
- **Idioma**: Padrão é português (`pt`). Escolha outro com `--language <código>` (ex.: `en`, `pt`, `es`)
- **Arquivos temporários**: O programa usa `~/.transcribe-workdir/` pra arquivos temporários mas limpa tudo quando termina
- **Qualidade do áudio**: Áudio melhor = transcrições melhores. O programa faz o possível pra limpar o áudio, mas lixo entra, lixo sai!
- **Tempo de processamento**: Transcrição leva tempo. Um vídeo de 1 hora pode levar uns 10-15 minutos dependendo da sua máquina

Boas transcrições! 🎉