# 🎬 Video Transcriber

Hey there! This is a handy Rust tool that takes your video files and turns them into text transcriptions. Perfect for when you need to convert lectures, meetings, or any video content into readable text.

**Why Rust?** This baby runs about 4x faster than the same thing written in Python. Because who wants to wait around for transcriptions, right?

## What does it do?

This little program is like having a personal Portuguese transcriptionist in your terminal. Here's the magic it performs:

1. **Takes your video file** (MP4 format only for now) and extracts the audio from it
2. **Cleans up the audio** by removing those awkward silences (you know, the ones that make files unnecessarily long)
3. **Chops up long videos** into manageable 30-minute chunks (because nobody wants to process a 3-hour file in one go)
4. **Transcribes everything to Portuguese** using OpenAI's Whisper model - it's pretty smart!
5. **Saves nice text files** with all the transcribed content in Portuguese

The best part? It cleans up after itself, removing all those temporary files it created during the process. No mess left behind! 🧹

## Prerequisites

Before you dive in, you'll need:

- **Rust** installed on your machine (obviously!)
- **FFmpeg** - This is super important! The program uses FFmpeg to handle all the audio/video processing
  - On macOS: `brew install ffmpeg`
  - On Ubuntu/Debian: `sudo apt install ffmpeg`
  - On Windows: Download from [ffmpeg.org](https://ffmpeg.org)

⚠️ **Note**: This tool has only been tested on macOS so far. It should work on Linux and Windows too, but your mileage may vary!

## Installation

Clone this bad boy and build it:

```bash
git clone <your-repo-url>
cd transcribe
cargo build --release
```

## How to use it

It's dead simple! Just run:

```bash
cargo run -- --input-video your-video.mp4 --output-dir transcriptions
```

Or if you built the release version:

```bash
./target/release/transcribe --input-video your-video.mp4 --output-dir transcriptions
```

### Real example

Say you've got a lecture recording called `physics_101.mp4`:

```bash
cargo run -- --input-video physics_101.mp4 --output-dir physics_notes
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

- **Video format**: Currently only accepts MP4 files. Got an AVI or MKV? You'll need to convert it first!
- **Language**: Currently hardcoded to Portuguese (pt) transcription only. If you need English or other languages, you'll have to modify the code
- **Temp files**: The program uses `~/.transcribe-workdir/` for temporary files but cleans everything up when done
- **Audio quality**: Better audio = better transcriptions. The program does its best to clean up the audio, but garbage in, garbage out!
- **Processing time**: Transcription takes time. A 1-hour video might take 10-15 minutes depending on your machine

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

1. **Pega seu arquivo de vídeo** (só formato MP4 por enquanto) e extrai o áudio dele
2. **Limpa o áudio** removendo aqueles silêncios chatos (sabe aqueles que deixam os arquivos desnecessariamente longos?)
3. **Corta vídeos longos** em pedaços de 30 minutos (porque ninguém quer processar um arquivo de 3 horas de uma vez)
4. **Transcreve tudo pra português** usando o modelo Whisper da OpenAI - é bem esperto!
5. **Salva arquivos de texto bonitinhos** com todo o conteúdo transcrito em português

A melhor parte? Ele limpa toda a bagunça depois, removendo todos os arquivos temporários que criou durante o processo. Não deixa sujeira pra trás! 🧹

### Pré-requisitos

Antes de começar, você vai precisar:

- **Rust** instalado na sua máquina (óbvio!)
- **FFmpeg** - Isso é super importante! O programa usa o FFmpeg pra lidar com todo o processamento de áudio/vídeo
  - No macOS: `brew install ffmpeg`
  - No Ubuntu/Debian: `sudo apt install ffmpeg`
  - No Windows: Baixe em [ffmpeg.org](https://ffmpeg.org)

⚠️ **Nota**: Essa ferramenta só foi testada no macOS até agora. Deve funcionar no Linux e Windows também, mas vai por sua conta e risco!

### Como usar

É moleza! Só rodar:

```bash
cargo run -- --input-video seu-video.mp4 --output-dir transcricoes
```

Ou se você compilou a versão release:

```bash
./target/release/transcribe --input-video seu-video.mp4 --output-dir transcricoes
```

### Exemplo real

Digamos que você tem uma gravação de aula chamada `fisica_101.mp4`:

```bash
cargo run -- --input-video fisica_101.mp4 --output-dir anotacoes_fisica
```

Isso vai:
- Criar um diretório `anotacoes_fisica` (se não existir)
- Extrair e processar o áudio
- Gerar arquivos de transcrição tipo `transcricao_parte_000.wav.txt`, `transcricao_parte_001.wav.txt`, etc.

### Primeira vez rodando?

Não se assuste se a primeira execução demorar! O programa precisa baixar o modelo Whisper (uns 500MB) na primeira vez. Ele vai salvar em `~/.whisper-model/` então as próximas execuções serão bem mais rápidas.

### Bom saber

- **Formato de vídeo**: Atualmente só aceita arquivos MP4. Tem um AVI ou MKV? Vai precisar converter primeiro!
- **Idioma**: Atualmente só transcreve em português (pt). Se precisar de inglês ou outros idiomas, vai ter que mexer no código
- **Arquivos temporários**: O programa usa `~/.transcribe-workdir/` pra arquivos temporários mas limpa tudo quando termina
- **Qualidade do áudio**: Áudio melhor = transcrições melhores. O programa faz o possível pra limpar o áudio, mas lixo entra, lixo sai!
- **Tempo de processamento**: Transcrição leva tempo. Um vídeo de 1 hora pode levar uns 10-15 minutos dependendo da sua máquina

Boas transcrições! 🎉