//! Распознавание речи: whisper.cpp на процессоре.
//!
//! Почему процессор: у whisper.cpp для Windows x64 нет сборки на Vulkan, а в CUDA-архиве
//! нет cuBLAS — без установленного CUDA Toolkit он тихо считает на процессоре
//! (замер в docs/phase-3.md). Модель small на 12 ядрах расшифровывает в 3,8 раза
//! быстрее реального времени — для диктовки и записей этого хватает.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

/// Модель распознавания. small, а не turbo: turbo пишет чище, но на процессоре втрое
/// медленнее — фраза в 10 секунд распознавалась бы 9–19 секунд. base быстрее, но по-русски
/// ошибается заметно чаще.
pub const MODEL_REPO: &str = "ggerganov/whisper.cpp";
pub const MODEL_NAME: &str = "ggml-small.bin";
pub const MODEL_SIZE: u64 = 487_601_967;
pub const MODEL_SHA256: &str = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";

/// Где лежит модель: там же, куда её положил бы каталог.
pub fn model_path(root: &Path) -> PathBuf {
    root.join("models").join("ggerganov").join("whisper.cpp").join(MODEL_NAME)
}

/// Потоков — по числу физических ядер: на Xeon E5-2678 v3 (12 ядер) 12 потоков
/// дали 18,9 с против 21,7 с у восьми; гиперпотоки whisper не ускоряют.
fn threads() -> usize {
    sysinfo::System::physical_core_count()
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get() / 2))
        .unwrap_or(4)
        .clamp(1, 16)
}

/// Окно кодировщика под длину записи. whisper всегда считает окно в 30 секунд
/// (1500 шагов), и фраза в 10 секунд стоила бы как полминуты записи. С окном по длине
/// фраза распознаётся вдвое быстрее без потери качества (docs/phase-3.md); две секунды
/// запаса — чтобы не обрезать конец. Длинные записи — как есть.
fn audio_ctx(seconds: f64) -> Option<u32> {
    if seconds >= 28.0 {
        return None;
    }
    let steps = ((seconds + 2.0) / 30.0 * 1500.0 / 64.0).ceil() as u32 * 64;
    Some(steps.clamp(256, 1500))
}

fn args(model: &Path, audio: &Path, seconds: Option<f64>) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        model.display().to_string(),
        "-f".into(),
        audio.display().to_string(),
        // Интерфейс русский — и говорят в нём по-русски. «auto» на коротких фразах
        // путает русский с украинским и болгарским.
        "-l".into(),
        "ru".into(),
        "-t".into(),
        threads().to_string(),
        // Только текст, без отметок времени; ход работы — в stderr.
        "-nt".into(),
        "-np".into(),
        "-pp".into(),
    ];
    if let Some(ac) = seconds.and_then(audio_ctx) {
        a.extend(["-ac".into(), ac.to_string()]);
    }
    a
}

/// Длина WAV из диктовки: 16 кГц, моно, 16 бит — 32 000 байт в секунду.
pub fn wav_seconds(bytes: usize) -> f64 {
    bytes.saturating_sub(44) as f64 / 32_000.0
}

/// Расшифровка файла. `seconds` — длина записи, если известна (диктовка): по ней
/// подбирается окно. `on_progress` — проценты, как их печатает whisper.
pub async fn transcribe(
    exe: &Path,
    model: &Path,
    audio: &Path,
    seconds: Option<f64>,
    cancel: &CancellationToken,
    on_progress: impl Fn(u8),
) -> Result<String, String> {
    crate::vcrt::prepare(exe)?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args(model, audio, seconds))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // Консольная программа из оконной — без этого мелькнёт чёрное окно.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("движок распознавания не запустился: {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let out = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    // Последние строки журнала — для «Подробностей», если что-то пойдёт не так.
    let mut tail: Vec<String> = vec![];
    loop {
        tokio::select! {
            line = stderr.next_line() => match line {
                Ok(Some(l)) => {
                    if let Some(p) = progress_of(&l) {
                        on_progress(p);
                    }
                    tail.push(l);
                    if tail.len() > 20 {
                        tail.remove(0);
                    }
                }
                _ => break,
            },
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(crate::llm::CANCELLED.into());
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.await.unwrap_or_default()).into_owned();
    if !status.success() {
        let log = tail.join("\n");
        return Err(if log.contains("failed to read audio") || log.contains("failed to decode") {
            "запись не читается — возможно, файл повреждён".into()
        } else {
            format!("распознавание не удалось:\n{log}")
        });
    }
    Ok(tidy(&text))
}

/// `whisper_print_progress_callback: progress =  45%` → 45.
fn progress_of(line: &str) -> Option<u8> {
    let rest = line.split("progress =").nth(1)?;
    // whisper считает проценты по кускам и к концу бывает «125%».
    rest.trim().trim_end_matches('%').trim().parse::<u8>().ok().map(|p| p.min(100))
}

/// Строки whisper — это куски по времени, а не абзацы: склеиваем в сплошной текст.
/// Пустые куски и метки вроде «[МУЗЫКА]» убираем: в разговоре с моделью они лишние.
fn tidy(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !(l.starts_with('[') && l.ends_with(']')))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_follows_phrase_length() {
        // 10 секунд — окно 640 (замер: 600 вдвое быстрее полного и без потери качества).
        assert_eq!(audio_ctx(10.0), Some(640));
        assert_eq!(audio_ctx(1.0), Some(256));
        assert_eq!(audio_ctx(27.0), Some(1472));
        assert_eq!(audio_ctx(60.0), None);
        assert_eq!(wav_seconds(44 + 320_000), 10.0);
    }

    #[test]
    fn args_are_russian_text_only() {
        let a = args(Path::new("m.bin"), Path::new("a.wav"), Some(10.0)).join(" ");
        assert!(a.contains("-l ru") && a.contains("-nt") && a.contains("-ac 640"), "{a}");
        assert!(!args(Path::new("m.bin"), Path::new("a.mp3"), None).join(" ").contains("-ac"));
    }

    #[test]
    fn progress_and_text_cleanup() {
        assert_eq!(progress_of("whisper_print_progress_callback: progress =  45%"), Some(45));
        assert_eq!(progress_of("whisper_init: loading model"), None);
        assert_eq!(tidy(" Первая часть.\n\n [МУЗЫКА]\n Вторая часть.\n"), "Первая часть. Вторая часть.");
    }

    /// Настоящий whisper.cpp из D:\Ollivo: минута Чехова (LibriVox) и фраза с окном по длине.
    /// `cargo test speech::tests::real_transcribe -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_transcribe() {
        let root = Path::new(r"D:\Ollivo");
        let engine = crate::engines::installed(root, "whisper.cpp").pop().expect("whisper.cpp не установлен");
        let model = model_path(root);
        let audio = root.join(r"lab\whisper\ch00.mp3");
        let started = std::time::Instant::now();
        let last = std::sync::atomic::AtomicU8::new(0);
        let text = transcribe(&engine.exe, &model, &audio, None, &CancellationToken::new(), |p| {
            last.store(p, std::sync::atomic::Ordering::Relaxed)
        })
        .await
        .unwrap();
        println!("72 с записи — {:.1} с, прогресс дошёл до {}%\n{text}", started.elapsed().as_secs_f64(), last.into_inner());
        assert!(text.contains("Человек в футляре") || text.contains("Чехова"), "{text}");
    }
}
