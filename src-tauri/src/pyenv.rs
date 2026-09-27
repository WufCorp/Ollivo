//! Окружение для картинок: свой Python с torch и зависимостями ComfyUI.
//!
//! Почему так устроено:
//! - **Без venv и без папок uv.** venv хранит абсолютный путь к Python в `pyvenv.cfg`,
//!   uv кладёт рядом с Python junction с абсолютным путём — после переноса папки программы
//!   ломаются оба (проверено на окружении фазы 0). Python из python-build-standalone находит
//!   свои библиотеки относительно `python.exe`, поэтому пакеты ставим прямо в его копию.
//! - **torch качает наш загрузчик** в 8 потоков со сверкой SHA256: uv тянул бы 2,6 ГБ в один
//!   поток ~40 минут (фаза 0). Остальное ставит uv, версии — на дату `exclude_newer`.
//! - **Веб-интерфейс ComfyUI не ставим** (~0,5 ГБ и большая часть времени установки):
//!   ComfyUI запускается с `--front-end-root` на пустую папку, API работает (проверено на 0.37.0).
//! - Всё временное (кэш uv, TEMP) — в `downloads\` папки программы: на системном диске
//!   у людей бывает 5 ГБ свободных, а окружение весит ~5.
//!
//! Раскладка:
//! ```text
//! engines\uv\…, engines\comfyui\…          — обычные движки из манифеста
//! engines\python\<версия>-<сборка>\         — Python с пакетами, `python.exe` в корне
//! engines\python\<версия>-<сборка>\ollivo.json — метка «готово»: что поставлено
//! ```

use crate::download::{self, Downloader, Phase};
use crate::engines::{self, InstallProgress, Stage};
use crate::hardware::Build;
use crate::manifest::{EngineBuild, Manifest, PythonSpec};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub const UV: &str = "uv";
pub const COMFY: &str = "comfyui";
const MARKER: &str = "ollivo.json";
/// Сколько окружение занимает сверх скачанного: распакованный torch ~4 ГБ из 2,6 ГБ
/// колеса, пакеты ComfyUI ~0,6 ГБ (замер фазы 0: venv 5,2 ГБ).
const UNPACKED_EXTRA: u64 = 3 << 30;

/// Готовое окружение.
#[derive(Debug, Clone, Serialize)]
pub struct Env {
    pub python: PathBuf,
    /// Папка ComfyUI (`main.py` внутри).
    pub comfy: PathBuf,
    pub build: Build,
}

/// Метка в папке окружения. Путей не хранит: папку программы могут перенести.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Marker {
    python: String,
    build: Build,
    /// Колёса torch, с которыми поставлено: сменились в манифесте — окружение новое.
    torch: Vec<String>,
    /// Версия ComfyUI, под чей `requirements.txt` поставлены пакеты.
    comfy: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("картинкам нужна видеокарта NVIDIA")]
    NoGpu,
    #[error("в манифесте нет окружения для картинок")]
    NoSpec,
    #[error(transparent)]
    Engine(#[from] engines::Error),
    /// Шаг установки не удался; `log` — хвост его вывода.
    #[error("{step}: {log}")]
    Step { step: &'static str, log: String },
}

impl Error {
    /// Ошибка сети внутри uv: PyPI или GitHub не ответили. Для кнопки «Повторить»
    /// и совета про прокси, как у обычных загрузок.
    pub fn is_net(&self) -> bool {
        let Error::Step { log, .. } = self else { return false };
        let log = log.to_lowercase();
        ["error sending request", "failed to fetch", "dns error", "timed out", "connection", "network"]
            .iter()
            .any(|s| log.contains(s))
    }
}

fn failed(step: &'static str) -> impl Fn(std::io::Error) -> Error {
    move |e| Error::Step { step, log: e.to_string() }
}

pub fn env_dir(root: &Path, spec: &PythonSpec, build: Build) -> PathBuf {
    let build = serde_json::to_value(build).unwrap();
    root.join("engines").join("python").join(format!("{}-{}", spec.version, build.as_str().unwrap()))
}

fn read_marker(dir: &Path) -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()
}

fn wanted_marker(spec: &PythonSpec, torch: &EngineBuild, comfy: &str) -> Marker {
    Marker {
        python: spec.version.clone(),
        build: torch.build,
        torch: torch.files.iter().map(|f| f.name.clone()).collect(),
        comfy: comfy.into(),
    }
}

/// Папка ComfyUI: движок текущей версии, `main.py` внутри.
fn comfy_dir(root: &Path, m: &Manifest) -> Option<PathBuf> {
    let e = m.engine(COMFY)?;
    let found = engines::installed(root, COMFY).into_iter().find(|i| i.version == e.version)?;
    found.exe.parent().map(Path::to_path_buf)
}

/// Готовое окружение под этот ПК или `None`, если чего-то не хватает.
pub fn ready(root: &Path, m: &Manifest, hw: Build) -> Option<Env> {
    let spec = m.python.as_ref()?;
    let torch = spec.torch_for(hw)?;
    let comfy = comfy_dir(root, m)?;
    let dir = env_dir(root, spec, hw);
    let want = wanted_marker(spec, torch, &m.engine(COMFY)?.version);
    (read_marker(&dir)? == want && dir.join("python.exe").is_file()).then(|| Env {
        python: dir.join("python.exe"),
        comfy,
        build: hw,
    })
}

/// Сколько осталось скачать и сколько места нужно всего. Для экрана «Картинки» до установки.
#[derive(Debug, Clone, Serialize)]
pub struct Needs {
    pub download: u64,
    pub disk: u64,
}

pub fn needs(root: &Path, m: &Manifest, hw: Build) -> Result<Needs, Error> {
    let spec = m.python.as_ref().ok_or(Error::NoSpec)?;
    let torch = spec.torch_for(hw).ok_or(Error::NoGpu)?;
    let mut download = 0;
    for id in [UV, COMFY] {
        let e = m.engine(id).ok_or(Error::NoSpec)?;
        if !engines::installed(root, id).iter().any(|i| i.version == e.version) {
            download += e.builds.first().map_or(0, |b| b.size());
        }
    }
    if read_marker(&env_dir(root, spec, hw)).is_none_or(|mk| mk.torch != wanted_marker(spec, torch, "").torch) {
        download += torch.size();
    }
    Ok(Needs { download, disk: download + if download > 0 { UNPACKED_EXTRA } else { 0 } })
}

/// Ставит всё для картинок: uv, ComfyUI, Python, torch, пакеты. Что уже стоит — пропускает.
/// Если сменилась только версия ComfyUI — досинхронизирует пакеты в том же окружении.
pub async fn install(
    dl: &Downloader,
    root: &Path,
    m: &Manifest,
    hw: Build,
    proxy: Option<String>,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(InstallProgress) + Send + Sync),
) -> Result<Env, Error> {
    let spec = m.python.as_ref().ok_or(Error::NoSpec)?;
    let torch = spec.torch_for(hw).ok_or(Error::NoGpu)?;
    let total = needs(root, m, hw)?.download;
    let mut offset = 0;

    // uv и ComfyUI — обычные движки: докачка, SHA256, «Починить».
    let mut installed = vec![];
    for id in [UV, COMFY] {
        let e = m.engine(id).ok_or(Error::NoSpec)?;
        let b = e.builds.first().ok_or(Error::NoSpec)?;
        let fresh = !engines::installed(root, id).iter().any(|i| i.version == e.version);
        let base = offset;
        let map = |p: InstallProgress| on_progress(InstallProgress { done: base + p.done, total, ..p });
        installed.push(engines::install(dl, root, e, b, cancel, &map).await?);
        if fresh {
            offset += b.size();
        }
    }
    let (uv, comfy) = (&installed[0].exe, installed[1].exe.parent().unwrap().to_path_buf());

    let dir = env_dir(root, spec, hw);
    let want = wanted_marker(spec, torch, &m.engine(COMFY).unwrap().version);
    let have = read_marker(&dir).filter(|mk| mk.python == want.python && mk.torch == want.torch);
    let downloads = root.join("downloads");
    let run = Runner::new(uv, root, proxy, cancel);

    // Окружение с нужным torch уже есть (обновился только ComfyUI) — доставляем пакеты в него.
    // Иначе собираем заново. Сразу в итоговой папке, без переименования в конце: 4,8 ГБ свежих
    // файлов антивирус держит открытыми, и Windows минуту-другую не даёт переименовать папку.
    // Оборванная установка готовой не выглядит: метка пишется последней.
    let mut wheels = vec![];
    if have.is_none() {
        for f in &torch.files {
            let req = download::Request {
                urls: f.urls.clone(),
                dest: downloads.join("wheels").join(&f.name),
                sha256: Some(f.sha256.clone()),
                connections: 8,
                chunk_size: 8 << 20,
            };
            let base = offset;
            let map = |p: download::Progress| {
                let stage = if p.phase == Phase::Verifying { Stage::Verify } else { Stage::Download };
                on_progress(InstallProgress { stage, done: base + p.done, total, speed: p.speed });
            };
            dl.download(&req, cancel, &map).await.map_err(engines::Error::from)?;
            offset += f.size;
            wheels.push(req.dest);
        }

        on_progress(InstallProgress { stage: Stage::Python, done: total, total, speed: 0.0 });
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(failed("установка Python"))?;
        }
        let pythons = downloads.join("python");
        let _ = std::fs::remove_dir_all(&pythons);
        run.uv(
            "установка Python",
            &["python", "install", &spec.version, "--install-dir", &path_arg(&pythons), "--no-bin", "--no-registry"],
        )
        .await?;
        let got = find_python(&pythons, &spec.version)
            .ok_or_else(|| Error::Step { step: "установка Python", log: "uv не положил Python".into() })?;
        std::fs::create_dir_all(dir.parent().unwrap()).map_err(failed("установка Python"))?;
        engines::rename_dir(&got, &dir).await.map_err(failed("установка Python"))?;
        let _ = std::fs::remove_dir_all(&pythons);
        // Пометка «ставить пакеты только через менеджер системы». Этот Python — только наш.
        let _ = std::fs::remove_file(dir.join("Lib").join("EXTERNALLY-MANAGED"));
    }
    let python = dir.join("python.exe");

    on_progress(InstallProgress { stage: Stage::Packages, done: total, total, speed: 0.0 });
    let reqs = std::fs::read_to_string(comfy.join("requirements.txt")).map_err(failed("список пакетов ComfyUI"))?;
    let reqs_file = downloads.join("comfy-requirements.txt");
    std::fs::write(&reqs_file, filter_requirements(&reqs, &spec.skip)).map_err(failed("список пакетов ComfyUI"))?;
    let mut args = vec![
        "pip".to_string(),
        "install".into(),
        "--python".into(),
        path_arg(&python),
        "--exclude-newer".into(),
        spec.exclude_newer.clone(),
        // Сразу компилируем .pyc: иначе первый запуск ComfyUI шёл 73 с вместо 9 (фаза 0).
        "--compile-bytecode".into(),
        // Копии, а не жёсткие ссылки на кэш: кэш удаляем, окружение переносят на другой диск.
        "--link-mode".into(),
        "copy".into(),
    ];
    args.extend(wheels.iter().map(|w| path_arg(w)));
    args.extend(["-r".into(), path_arg(&reqs_file)]);
    run.uv("установка пакетов", &args.iter().map(String::as_str).collect::<Vec<_>>()).await?;

    on_progress(InstallProgress { stage: Stage::Warmup, done: total, total, speed: 0.0 });
    run.cmd(&python, "подготовка ComfyUI", &["-E", "-s", "-m", "compileall", "-q", "-j", "0", &path_arg(&comfy)]).await?;
    let out = run
        .cmd(&python, "проверка", &["-E", "-s", "-c", "import torch; print('cuda', torch.cuda.is_available())"])
        .await?;
    if !out.contains("cuda True") {
        return Err(Error::Step { step: "проверка", log: format!("torch не видит видеокарту: {}", out.trim()) });
    }

    std::fs::write(dir.join(MARKER), serde_json::to_vec_pretty(&want).unwrap()).map_err(failed("метка"))?;
    // Колёса — все из манифеста, а не только скачанные сейчас: после оборванной установки
    // окружение могло доставиться без них, а 2,6 ГБ лежали бы в `downloads` до «Очистить».
    for f in spec.torch.iter().flat_map(|b| &b.files) {
        let _ = std::fs::remove_file(downloads.join("wheels").join(&f.name));
    }
    let _ = std::fs::remove_dir(downloads.join("wheels"));
    let _ = std::fs::remove_file(&reqs_file);
    // Кэш uv после установки не нужен, а весит почти как окружение.
    let _ = std::fs::remove_dir_all(downloads.join("uv-cache"));
    let _ = std::fs::remove_dir_all(downloads.join("tmp"));
    Ok(Env { python, comfy, build: hw })
}

/// Путь аргументом для uv и Python. Кириллица проходит как есть: Rust передаёт командную
/// строку в UTF-16.
fn path_arg(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Папка Python, которую положил `uv python install`: `cpython-<версия>-windows-…`.
/// Рядом uv кладёт junction `cpython-3.12-…` на неё же — его не берём: переименовать
/// нужно настоящую папку.
fn find_python(dir: &Path, version: &str) -> Option<PathBuf> {
    let prefix = format!("cpython-{version}-");
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        name.starts_with(&prefix) && p.join("python.exe").is_file()
    })
}

/// `requirements.txt` ComfyUI без пакетов из `skip` и без комментариев.
fn filter_requirements(text: &str, skip: &[String]) -> String {
    let norm = |s: &str| s.trim().to_lowercase().replace('_', "-");
    let skip: Vec<String> = skip.iter().map(|s| norm(s)).collect();
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let name = l.split(|c: char| "<>=!~;[ ".contains(c)).next().unwrap_or("");
            !skip.contains(&norm(name))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

/// Запуск uv и Python для установки: без окна, вывод — в `logs\images-install.log`,
/// временные файлы — в папке программы, чужие настройки uv, pip и Python не действуют.
struct Runner<'a> {
    uv: &'a Path,
    log: PathBuf,
    env: Vec<(String, String)>,
    cancel: &'a CancellationToken,
}

impl<'a> Runner<'a> {
    fn new(uv: &'a Path, root: &Path, proxy: Option<String>, cancel: &'a CancellationToken) -> Self {
        let downloads = root.join("downloads");
        let tmp = path_arg(&downloads.join("tmp"));
        let mut env = vec![
            ("UV_CACHE_DIR".into(), path_arg(&downloads.join("uv-cache"))),
            ("UV_NO_CONFIG".into(), "1".into()),
            ("UV_PYTHON_INSTALL_DIR".into(), path_arg(&downloads.join("python"))),
            ("TEMP".into(), tmp.clone()),
            ("TMP".into(), tmp),
            // Вывод Python в логе — в UTF-8, а не в кодировке консоли.
            ("PYTHONUTF8".into(), "1".into()),
        ];
        // Прокси — из настроек Ollivo, как у всех загрузок программы, а не системный.
        if let Some(p) = proxy {
            for k in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"] {
                env.push((k.into(), p.clone()));
            }
        }
        Self { uv, log: root.join("logs").join("images-install.log"), env, cancel }
    }

    async fn uv(&self, step: &'static str, args: &[&str]) -> Result<String, Error> {
        self.cmd(self.uv, step, args).await
    }

    /// Выполняет команду и возвращает её вывод; ошибка — с хвостом вывода.
    async fn cmd(&self, exe: &Path, step: &'static str, args: &[&str]) -> Result<String, Error> {
        let _ = std::fs::create_dir_all(self.log.parent().unwrap());
        let _ = std::fs::create_dir_all(self.env.iter().find(|(k, _)| k == "TEMP").map(|(_, v)| v).unwrap());
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        // Чужие окружения человека (Anaconda, свой uv, PYTHONPATH) не должны влиять на наш Python.
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy().to_uppercase();
            if k.starts_with("UV_") || k.starts_with("PIP_") || k.starts_with("PYTHON") || k == "VIRTUAL_ENV" || k.starts_with("CONDA") {
                cmd.env_remove(&k);
            }
        }
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let child = cmd.spawn().map_err(failed(step))?;
        let out = tokio::select! {
            out = child.wait_with_output() => out.map_err(failed(step))?,
            _ = self.cancel.cancelled() => return Err(engines::Error::Download(download::Error::Cancelled).into()),
        };
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        if let Ok(mut f) = tokio::fs::OpenOptions::new().create(true).append(true).open(&self.log).await {
            let head = format!("\n=== {step}: {} {}\n", exe.file_name().unwrap_or_default().to_string_lossy(), args.join(" "));
            let _ = f.write_all(head.as_bytes()).await;
            let _ = f.write_all(text.as_bytes()).await;
        }
        if !out.status.success() {
            let tail: Vec<&str> = text.lines().rev().take(15).collect();
            return Err(Error::Step { step, log: tail.into_iter().rev().collect::<Vec<_>>().join("\n") });
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_without_web_ui() {
        let text = "comfyui-frontend-package==1.52.7\ncomfyui_workflow_templates==0.11.66\ntorch\n\
                    numpy>=1.25.0\n\n#non essential dependencies:\nkornia>=0.7.1 # upscale\nPyOpenGL>=3.1.8\n\
                    pydantic~=2.0\n";
        let skip = ["comfyui-frontend-package", "comfyui-workflow-templates", "pyopengl"].map(String::from);
        assert_eq!(filter_requirements(text, &skip), "torch\nnumpy>=1.25.0\nkornia>=0.7.1\npydantic~=2.0\n");
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = crate::testserver::tmp().join(format!("Иван Петров {}-py-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn picks_real_python_dir_not_minor_link() {
        let dir = tmp("find");
        for d in ["cpython-3.12-windows-x86_64-none", "cpython-3.12.14-windows-x86_64-none", ".temp"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("cpython-3.12.14-windows-x86_64-none").join("python.exe"), b"").unwrap();
        let got = find_python(&dir, "3.12.14").unwrap();
        assert!(got.ends_with("cpython-3.12.14-windows-x86_64-none"));
        assert!(find_python(&dir, "3.12.15").is_none());
    }

    /// Окружение «готово» только с тем же torch и под ту же версию ComfyUI; путей метка
    /// не хранит — перенос папки программы ей не мешает.
    #[test]
    fn ready_needs_matching_marker() {
        let root = tmp("ready");
        let m = Manifest::bundled();
        let spec = m.python.as_ref().unwrap();
        assert!(ready(&root, &m, Build::Cuda12).is_none());
        assert!(matches!(needs(&root, &m, Build::Vulkan), Err(Error::NoGpu)));
        let n = needs(&root, &m, Build::Cuda12).unwrap();
        assert!(n.download > 2_000_000_000 && n.disk > n.download);

        // ComfyUI «стоит».
        let comfy = m.engine(COMFY).unwrap();
        let cdir = engines::engine_dir(&root, comfy, Build::Cpu);
        std::fs::create_dir_all(cdir.join("ComfyUI-0.37.0")).unwrap();
        std::fs::write(cdir.join(&comfy.exe), b"").unwrap();
        let mark = serde_json::json!({"id": COMFY, "version": comfy.version, "build": "cpu",
            "dir": cdir, "exe": cdir.join(&comfy.exe)});
        std::fs::write(cdir.join("ollivo.json"), mark.to_string()).unwrap();

        let dir = env_dir(&root, spec, Build::Cuda12);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("python.exe"), b"").unwrap();
        let mut mk = wanted_marker(spec, spec.torch_for(Build::Cuda12).unwrap(), "0.36.0");
        std::fs::write(dir.join(MARKER), serde_json::to_vec(&mk).unwrap()).unwrap();
        assert!(ready(&root, &m, Build::Cuda12).is_none(), "пакеты поставлены под старый ComfyUI");
        // Но torch тот же — качать его заново не нужно.
        assert!(needs(&root, &m, Build::Cuda12).unwrap().download < 100_000_000);

        mk.comfy = comfy.version.clone();
        std::fs::write(dir.join(MARKER), serde_json::to_vec(&mk).unwrap()).unwrap();
        let env = ready(&root, &m, Build::Cuda12).unwrap();
        assert_eq!(env.python, dir.join("python.exe"));
        assert!(env.comfy.ends_with("ComfyUI-0.37.0"));
        assert!(ready(&root, &m, Build::Cuda13).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn net_errors_from_uv() {
        let e = Error::Step { step: "установка пакетов", log: "error: Failed to fetch: `https://pypi.org/simple/numpy/`".into() };
        assert!(e.is_net());
        assert!(!Error::Step { step: "проверка", log: "ImportError".into() }.is_net());
    }

    /// Настоящая установка в `D:\Ollivo`: uv, ComfyUI, Python, torch, пакеты.
    /// Колесо torch из фазы 0 можно заранее положить в `D:\Ollivo\downloads\wheels` — тогда
    /// оно не качается заново (загрузчик доверяет готовому файлу).
    /// `cargo test pyenv::tests::install_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn install_real() {
        let root = Path::new(r"D:\Ollivo");
        let m = Manifest::bundled();
        let hw = crate::hardware::detect().cuda_build;
        println!("нужно: {:?}", needs(root, &m, hw).unwrap());
        let started = std::time::Instant::now();
        let last = std::sync::Mutex::new(None);
        let env = install(&Downloader::new(), root, &m, hw, None, &CancellationToken::new(), &|p| {
            let mut last = last.lock().unwrap();
            if *last != Some(p.stage) {
                println!("{:>6.0} с  {:?}", started.elapsed().as_secs_f64(), p.stage);
                *last = Some(p.stage);
            }
        })
        .await
        .unwrap();
        println!("готово за {:.0} с: {env:?}", started.elapsed().as_secs_f64());
        assert!(ready(root, &m, hw).is_some());
    }
}
