//! ffmpeg: то, что не читают движки. whisper.cpp понимает только WAV, MP3, FLAC и OGG
//! с Vorbis — а голосовые из Telegram (OGG с Opus), записи с телефона (M4A) и звук
//! из видео приходят в других форматах. Движок чата не открывает HEIC и AVIF — а в HEIC
//! снимает iPhone. Всё это умеет ffmpeg: ставится по требованию, как whisper, и работает
//! отдельной программой.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Id движка в манифесте.
pub const FFMPEG: &str = "ffmpeg";
/// Части, которые докачиваются по требованию (`parts_status` в `lib.rs`).
pub const WHISPER: &str = "whisper.cpp";
pub const SPEECH_MODEL: &str = "speech:model";

/// Записи и видео, которые whisper не читает: звук из них достаёт ffmpeg.
const FFMPEG_AUDIO: &[&str] = &[
    "m4a", "aac", "opus", "wma", "amr", "3gp", "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv",
];
/// Картинки, внутри которых кадр HEVC или AV1.
const FFMPEG_IMAGES: &[&str] = &["heic", "heif", "hif", "avif"];

/// Картинка из HEIC собирается за 1,4 с (4032×3024 из 48 плиток, Xeon E5-2678 v3);
/// дольше полуминуты — файл с подвохом.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Что нужно, чтобы приложить файл: пусто — ничего, иначе части по порядку установки.
pub fn parts_for(path: &Path) -> Vec<&'static str> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = ext.as_str();
    if FFMPEG_IMAGES.contains(&ext) {
        vec![FFMPEG]
    } else if FFMPEG_AUDIO.contains(&ext) || (matches!(ext, "ogg" | "oga") && is_opus(path)) {
        vec![FFMPEG, WHISPER, SPEECH_MODEL]
    } else if matches!(ext, "wav" | "mp3" | "flac" | "ogg" | "oga") {
        vec![WHISPER, SPEECH_MODEL]
    } else {
        vec![]
    }
}

/// Запись это или документ с картинкой: запись идёт в распознавание речи.
pub fn is_audio(path: &Path) -> bool {
    parts_for(path).contains(&SPEECH_MODEL)
}

/// OGG бывает с Vorbis (его whisper читает) и с Opus — так пишет голосовые Telegram.
/// Кодек назван в первом пакете, в первых сотне байт файла.
fn is_opus(path: &Path) -> bool {
    let mut head = [0u8; 128];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    head[..n].windows(8).any(|w| w == b"OpusHead")
}

fn command(ffmpeg: &Path) -> Command {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Консольная программа из оконной — без этого мелькнёт чёрное окно.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Ошибка ffmpeg человеческими словами; сырой журнал — в конце, для того, кто будет помогать.
fn explain(log: &str, what: &str) -> String {
    if log.contains("does not contain any stream") || log.contains("matches no streams") {
        format!("в файле нет {what}")
    } else if log.contains("Invalid data found") || log.contains("moov atom not found") {
        "файл не читается — возможно, он повреждён или записан не до конца".into()
    } else {
        format!("файл не удалось прочитать:\n{}", log.trim())
    }
}

/// Звук из записи или видео → WAV 16 кГц моно, как нужно whisper. Минута записи —
/// около 0,2 с (M4A, MP4), OPUS — 0,4 с. Видео не декодируется (`-vn`).
pub async fn to_wav(ffmpeg: &Path, input: &Path, out: &Path, cancel: &CancellationToken) -> Result<(), String> {
    let mut cmd = tokio::process::Command::from(command(ffmpeg));
    cmd.arg("-i")
        .arg(input)
        .args(["-vn", "-map", "0:a:0", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-y"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("ffmpeg не запустился: {e}"))?;
    let result = tokio::select! {
        r = child.wait_with_output() => r.map_err(|e| e.to_string())?,
        _ = cancel.cancelled() => return Err(crate::llm::CANCELLED.into()),
    };
    if result.status.success() {
        return Ok(());
    }
    let _ = std::fs::remove_file(out);
    Err(explain(&String::from_utf8_lossy(&result.stderr), "звука"))
}

/// HEIC, AVIF → PNG. Сетку плиток (так снимает iPhone) и поворот из файла ffmpeg
/// применяет сам: портрет 4032×3024 выходит 3024×4032.
pub fn to_png(ffmpeg: &Path, input: &Path) -> Result<Vec<u8>, String> {
    let mut child = command(ffmpeg)
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg не запустился: {e}"))?;
    // Читаем в отдельных потоках: картинка в 6 МБ забьёт канал, и ffmpeg встанет,
    // пока мы ждём его завершения.
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            buf
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > IMAGE_TIMEOUT {
            let _ = child.kill();
            return Err("картинка открывается слишком долго — возможно, она повреждена".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (png, log) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if !status.success() || png.is_empty() {
        return Err(explain(&String::from_utf8_lossy(&log), "картинки"));
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn what_each_file_needs() {
        let dir = std::env::temp_dir().join(format!("ollivo-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vorbis = dir.join("song.ogg");
        std::fs::write(&vorbis, b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\x1evorbis").unwrap();
        let voice = dir.join("voice.ogg");
        std::fs::write(&voice, b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\x13OpusHead").unwrap();
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(parts_for(&p("a.MP3")), vec![WHISPER, SPEECH_MODEL]);
        assert_eq!(parts_for(&vorbis), vec![WHISPER, SPEECH_MODEL]);
        assert_eq!(parts_for(&voice), vec![FFMPEG, WHISPER, SPEECH_MODEL]);
        assert_eq!(parts_for(&p("лекция.m4a")), vec![FFMPEG, WHISPER, SPEECH_MODEL]);
        assert_eq!(parts_for(&p("видео.mp4")), vec![FFMPEG, WHISPER, SPEECH_MODEL]);
        assert_eq!(parts_for(&p("IMG_0001.HEIC")), vec![FFMPEG]);
        assert!(parts_for(&p("фото.jpg")).is_empty() && parts_for(&p("договор.pdf")).is_empty());
        assert!(is_audio(&voice) && !is_audio(&p("x.avif")));
    }

    #[test]
    fn errors_in_plain_words() {
        assert_eq!(explain("[out#0/wav] Output file does not contain any stream", "звука"), "в файле нет звука");
        assert!(explain("moov atom not found\nError opening input", "звука").contains("повреждён"));
        assert!(explain("что-то новое", "звука").ends_with("что-то новое"));
    }

    /// Настоящий ffmpeg: `OLLIVO_FFMPEG=<ffmpeg.exe> OLLIVO_MEDIA=<папка> cargo test media::tests::real -- --ignored --nocapture`.
    /// В папке — `arrow.heic` (портрет с iPhone: сетка 4032×3024 и поворот, из тестов
    /// pillow_heif), `example.avif` (из libheif), `ch00.m4a`, `ch00.opus`, `ch00.mp4`
    /// (72 с речи) и `silent.mp4` (видео без звука).
    #[tokio::test]
    #[ignore]
    async fn real() {
        let ff = PathBuf::from(std::env::var("OLLIVO_FFMPEG").unwrap());
        let dir = PathBuf::from(std::env::var("OLLIVO_MEDIA").unwrap());
        for (name, w, h) in [("arrow.heic", 3024, 4032), ("example.avif", 800, 533)] {
            let started = Instant::now();
            let png = to_png(&ff, &dir.join(name)).unwrap();
            let img = image::load_from_memory(&png).unwrap();
            println!("{name}: {}×{} за {:.2} с", img.width(), img.height(), started.elapsed().as_secs_f64());
            assert_eq!((img.width(), img.height()), (w, h));
        }
        // Целиком, как из окна: HEIC → JPG в папке вложений.
        let a = crate::attach::read(&dir.join("arrow.heic"), &std::env::temp_dir().join("ollivo-media-img"), Some(&ff)).unwrap();
        let stored = a.path.unwrap();
        let img = image::open(&stored).unwrap();
        println!("{} — {} КБ", stored.display(), std::fs::metadata(&stored).unwrap().len() / 1024);
        assert_eq!((stored.extension().unwrap().to_str(), img.width(), img.height()), (Some("jpg"), 3024, 4032));

        let c = CancellationToken::new();
        for name in ["ch00.m4a", "ch00.opus", "ch00.mp4"] {
            let out = std::env::temp_dir().join(format!("{name}.wav"));
            let started = Instant::now();
            to_wav(&ff, &dir.join(name), &out, &c).await.unwrap();
            let seconds = crate::speech::wav_seconds(std::fs::metadata(&out).unwrap().len() as usize);
            println!("{name}: {seconds:.1} с звука за {:.2} с", started.elapsed().as_secs_f64());
            assert!((seconds - 72.0).abs() < 1.5, "{seconds}");
        }
        let err = to_wav(&ff, &dir.join("silent.mp4"), &std::env::temp_dir().join("silent.wav"), &c).await.unwrap_err();
        assert_eq!(err, "в файле нет звука");
    }
}
