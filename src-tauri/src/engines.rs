//! Установка движков по манифесту: скачать архивы, проверить, распаковать.
//!
//! Раскладка в папке данных (`<диск>:\Ollivo`):
//! ```text
//! downloads\<архив>.zip|.7z                — пока идёт установка, потом удаляется
//! engines\<id>\<версия>-<сборка>\          — готовый движок
//! engines\<id>\<версия>-<сборка>\ollivo.json — метка «установлено полностью»
//! engines\<id>\<версия>-<сборка>\ollivo.files.json — размеры и SHA256 файлов (для «Починить»)
//! ```
//! Распаковка идёт во временную папку `…-<сборка>.tmp` и переименовывается
//! только в конце, поэтому оборванная установка не выглядит готовой.

use crate::download::{self, Downloader, Phase};
use crate::hardware::Build;
use crate::manifest::{Engine, EngineBuild};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

const MARKER: &str = "ollivo.json";
const FILES: &str = "ollivo.files.json";

/// Файл движка в момент установки: по этому списку «Починить» ищет битое.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileSum {
    /// Путь внутри папки движка, через `/`.
    path: String,
    size: u64,
    sha256: String,
}

/// Итог «Починить».
#[derive(Debug, Clone, Serialize)]
pub struct Repair {
    #[serde(flatten)]
    pub installed: Installed,
    /// Пропавшие и изменённые файлы (пусто — всё было цело).
    pub broken: Vec<String>,
    /// Движок переустановлен.
    pub reinstalled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub build: Build,
    pub dir: PathBuf,
    pub exe: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Download,
    Verify,
    Unpack,
    /// Дальше — только у окружения для картинок (`pyenv.rs`): свой Python,
    Python,
    /// пакеты (сколько осталось, заранее не известно),
    Packages,
    /// подготовка ComfyUI к быстрому первому запуску.
    Warmup,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub stage: Stage,
    pub done: u64,
    pub total: u64,
    pub speed: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Download(#[from] download::Error),
    #[error("архив {0} не распаковался: {1}")]
    Unpack(String, String),
    #[error("в архиве нет {0}")]
    NoExe(String),
    #[error("диск: {0}")]
    Io(#[from] std::io::Error),
}

pub fn engine_dir(root: &Path, engine: &Engine, build: Build) -> PathBuf {
    let build = serde_json::to_value(build).unwrap();
    root.join("engines")
        .join(&engine.id)
        .join(format!("{}-{}", engine.version, build.as_str().unwrap()))
}

/// Все полностью установленные сборки движка (любых версий).
pub fn installed(root: &Path, id: &str) -> Vec<Installed> {
    let Ok(dirs) = std::fs::read_dir(root.join("engines").join(id)) else { return vec![] };
    dirs.filter_map(|d| {
        let here = d.ok()?.path();
        let json = std::fs::read(here.join(MARKER)).ok()?;
        serde_json::from_slice::<Installed>(&json).ok().map(|i| i.moved_to(&here))
    })
    .filter(|i| i.exe.exists())
    .collect()
}

impl Installed {
    /// Пометка хранит полные пути на момент установки, а папку программы могут
    /// перенести на другой диск. Пути считаем от того места, где сборка лежит сейчас.
    fn moved_to(mut self, here: &Path) -> Self {
        if let Ok(rel) = self.exe.strip_prefix(&self.dir) {
            self.exe = here.join(rel);
        }
        self.dir = here.to_path_buf();
        self
    }
}

pub async fn install(
    dl: &Downloader,
    root: &Path,
    engine: &Engine,
    build: &EngineBuild,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(InstallProgress) + Send + Sync),
) -> Result<Installed, Error> {
    let dir = engine_dir(root, engine, build.build);
    if let Ok(json) = std::fs::read(dir.join(MARKER)) {
        if let Ok(done) = serde_json::from_slice::<Installed>(&json) {
            return Ok(done);
        }
    }

    let total = build.size();
    let downloads = root.join("downloads");
    let mut archives = Vec::new();
    let mut offset = 0;
    for f in &build.files {
        let req = download::Request {
            urls: f.urls.clone(),
            dest: downloads.join(&f.name),
            sha256: Some(f.sha256.clone()),
            connections: 8,
            chunk_size: 8 << 20,
        };
        let map = |p: download::Progress| {
            let stage = if p.phase == Phase::Verifying { Stage::Verify } else { Stage::Download };
            on_progress(InstallProgress { stage, done: offset + p.done, total, speed: p.speed });
        };
        dl.download(&req, cancel, &map).await?;
        offset += f.size;
        archives.push((req.dest, f.only.clone()));
    }

    on_progress(InstallProgress { stage: Stage::Unpack, done: total, total, speed: 0.0 });
    let tmp = tmp_sibling(&dir);
    let (tmp2, archives2) = (tmp.clone(), archives.clone());
    let sums = tokio::task::spawn_blocking(move || {
        unpack_all(&archives2, &tmp2)?;
        Ok::<_, Error>(file_sums(&tmp2)?)
    })
    .await
    .map_err(|e| Error::Unpack(String::new(), e.to_string()))??;

    if !tmp.join(&engine.exe).is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(Error::NoExe(engine.exe.clone()));
    }
    let done = Installed {
        id: engine.id.clone(),
        version: engine.version.clone(),
        build: build.build,
        exe: dir.join(&engine.exe),
        dir: dir.clone(),
    };
    std::fs::write(tmp.join(FILES), serde_json::to_vec_pretty(&sums).unwrap())?;
    std::fs::write(tmp.join(MARKER), serde_json::to_vec_pretty(&done).unwrap())?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    rename_dir(&tmp, &dir).await?;
    for (a, _) in &archives {
        let _ = std::fs::remove_file(a);
    }
    Ok(done)
}

/// «Починить»: убирает остатки оборванных установок, сверяет файлы сборки
/// со списком, записанным при установке, и переустанавливает, если что-то не так.
/// Нет списка (поставлено старой версией Ollivo) — проверить нечем, переустанавливаем.
pub async fn repair(
    dl: &Downloader,
    root: &Path,
    engine: &Engine,
    build: &EngineBuild,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(InstallProgress) + Send + Sync),
) -> Result<Repair, Error> {
    clean_leftovers(&root.join("engines").join(&engine.id));
    let dir = engine_dir(root, engine, build.build);
    let marker = std::fs::read(dir.join(MARKER)).ok().and_then(|j| serde_json::from_slice::<Installed>(&j).ok());
    let broken = match marker {
        Some(done) if done.exe.is_file() => {
            let (dir2, cancel2) = (dir.clone(), cancel.clone());
            let (tx, mut rx) = tokio::sync::watch::channel((0u64, 0u64));
            let mut job = tokio::task::spawn_blocking(move || {
                check_files(&dir2, &cancel2, &|done, total| {
                    let _ = tx.send((done, total));
                })
            });
            let broken = loop {
                tokio::select! {
                    r = &mut job => break r.map_err(|e| Error::Unpack(String::new(), e.to_string()))??,
                    Ok(()) = rx.changed() => {
                        let (done, total) = *rx.borrow_and_update();
                        on_progress(InstallProgress { stage: Stage::Verify, done, total, speed: 0.0 });
                    }
                }
            };
            if cancel.is_cancelled() {
                return Err(download::Error::Cancelled.into());
            }
            if broken.is_empty() {
                return Ok(Repair { installed: done, broken, reinstalled: false });
            }
            broken
        }
        _ => vec![engine.exe.clone()],
    };
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let installed = install(dl, root, engine, build, cancel, on_progress).await?;
    Ok(Repair { installed, broken, reinstalled: true })
}

/// `<папка>.tmp` рядом с папкой сборки. Не `with_extension`: у «0.37.0-cpu» он счёл бы
/// расширением «0-cpu» и дал «0.37.tmp».
pub fn tmp_sibling(dir: &Path) -> PathBuf {
    let mut s = dir.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Переименование свежераспакованной папки с повторами: антивирус проверяет новые файлы
/// и держит их открытыми, Windows отвечает «Отказано в доступе». Замер: окружение картинок
/// (4,8 ГБ) не переименовалось сразу после установки, а через пару минут — без ошибок.
pub async fn rename_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < 120 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            r => return r,
        }
    }
}

/// Остатки оборванной распаковки: папки `…-<сборка>.tmp`.
fn clean_leftovers(engines: &Path) {
    let Ok(dirs) = std::fs::read_dir(engines) else { return };
    for d in dirs.flatten() {
        if d.path().extension().is_some_and(|e| e == "tmp") {
            let _ = std::fs::remove_dir_all(d.path());
        }
    }
}

/// Сверяет файлы с `ollivo.files.json`. Лишние файлы (логи, кеши движка) не трогаем.
/// Синхронная: хеширует сотни мегабайт, звать из `spawn_blocking`.
fn check_files(dir: &Path, cancel: &CancellationToken, report: &dyn Fn(u64, u64)) -> Result<Vec<String>, Error> {
    let Ok(json) = std::fs::read(dir.join(FILES)) else { return Ok(vec![FILES.into()]) };
    let Ok(sums) = serde_json::from_slice::<Vec<FileSum>>(&json) else { return Ok(vec![FILES.into()]) };
    let total = sums.iter().map(|f| f.size).sum();
    let (mut done, mut broken) = (0, vec![]);
    for f in &sums {
        if cancel.is_cancelled() {
            break;
        }
        let path = dir.join(&f.path);
        let ok = std::fs::metadata(&path).is_ok_and(|m| m.len() == f.size)
            && download::sha256_file(&path).is_ok_and(|h| h == f.sha256);
        if !ok {
            broken.push(f.path.clone());
        }
        done += f.size;
        report(done, total);
    }
    Ok(broken)
}

fn file_sums(dir: &Path) -> std::io::Result<Vec<FileSum>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<FileSum>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let path = e.path();
            if e.file_type()?.is_dir() {
                walk(base, &path, out)?;
            } else {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                out.push(FileSum { path: rel, size: e.metadata()?.len(), sha256: download::sha256_file(&path)? });
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Архивы и что из каждого взять (пусто — всё).
fn unpack_all(archives: &[(PathBuf, Vec<String>)], into: &Path) -> Result<(), Error> {
    if into.exists() {
        std::fs::remove_dir_all(into)?;
    }
    std::fs::create_dir_all(into)?;
    for (a, only) in archives {
        let name = a.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let bad = |e: &dyn std::fmt::Display| Error::Unpack(name.clone(), e.to_string());
        if a.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z")) {
            unpack_7z(a, only, into).map_err(|e| bad(&e))?;
            continue;
        }
        let mut zip = zip::ZipArchive::new(std::fs::File::open(a)?).map_err(|e| bad(&e))?;
        if only.is_empty() {
            // `extract` сам отбрасывает пути вида `..\..\` (zip slip).
            zip.extract(into).map_err(|e| bad(&e))?;
            continue;
        }
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| bad(&e))?;
            let dest = inside(into, entry.name()).filter(|_| entry.is_file() && wanted(only, entry.name()));
            if let Some(dest) = dest {
                write_entry(&dest, &mut entry)?;
            }
        }
    }
    Ok(())
}

/// 7z: сборки ffmpeg в нём втрое меньше, чем в zip (35 МБ против 115).
fn unpack_7z(archive: &Path, only: &[String], into: &Path) -> Result<(), sevenz_rust2::Error> {
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())?;
    reader.for_each_entries(|entry, data| {
        match inside(into, entry.name()).filter(|_| !entry.is_directory() && wanted(only, entry.name())) {
            Some(dest) => write_entry(&dest, data)?,
            // Сплошной архив читается подряд: ненужный файл всё равно дочитываем до конца,
            // иначе следующий начнётся с его хвоста.
            None => {
                std::io::copy(data, &mut std::io::sink())?;
            }
        }
        Ok(true)
    })
}

fn write_entry(dest: &Path, data: &mut dyn std::io::Read) -> std::io::Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::io::copy(data, &mut std::fs::File::create(dest)?)?;
    Ok(())
}

/// Путь из архива внутри папки движка; `None` — путь пустой или ведёт наружу
/// (`..`, `C:\`, `\\server`): архив с зеркала мог быть подменён.
fn inside(into: &Path, name: &str) -> Option<PathBuf> {
    let rel = Path::new(name);
    let normal = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    (normal && rel.components().next().is_some()).then(|| into.join(rel))
}

fn wanted(only: &[String], name: &str) -> bool {
    let name = name.replace('\\', "/");
    only.is_empty() || only.iter().any(|o| *o == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Папку программы перенесли — движок находится на новом месте.
    #[test]
    fn installed_survives_moving_data_dir() {
        let root = crate::testserver::tmp().join(format!("ollivo-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("engines").join("llama.cpp").join("b1-vulkan");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("llama-server.exe"), b"").unwrap();
        let old = Path::new(r"C:\Ollivo\engines\llama.cpp\b1-vulkan");
        let marker = Installed {
            id: "llama.cpp".into(),
            version: "b1".into(),
            build: Build::Vulkan,
            dir: old.to_path_buf(),
            exe: old.join("bin").join("llama-server.exe"),
        };
        std::fs::write(dir.join(MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
        let found = installed(&root, "llama.cpp");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].exe, dir.join("bin").join("llama-server.exe"));
        let _ = std::fs::remove_dir_all(&root);
    }
    use crate::manifest::EngineFile;
    use crate::testserver::{serve, sha};
    use std::io::Write;
    use std::sync::atomic::Ordering;

    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn engine(archives: &[(&str, &[u8])]) -> (Engine, Vec<crate::testserver::Server>) {
        let mut servers = vec![];
        let files = archives
            .iter()
            .map(|(name, data)| {
                let srv = serve(data.to_vec(), true, 0);
                let f = EngineFile {
                    name: name.to_string(),
                    urls: vec![srv.url.clone()],
                    sha256: sha(data),
                    size: data.len() as u64,
                    only: vec![],
                };
                servers.push(srv);
                f
            })
            .collect();
        let e = Engine {
            id: "llama.cpp".into(),
            title: "тест".into(),
            version: "b1".into(),
            exe: "llama-server.exe".into(),
            prefer: vec![Build::Vulkan],
            builds: vec![EngineBuild { build: Build::Vulkan, files }],
        };
        (e, servers)
    }

    fn tmp_root(name: &str) -> PathBuf {
        // Кириллица и пробел — как в папке программы у человека по имени «Иван Петров».
        let dir = crate::testserver::tmp().join(format!("Иван Петров {}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn installs_two_archives_into_one_dir() {
        let main = make_zip(&[("llama-server.exe", b"exe"), ("ggml.dll", b"dll")]);
        let rt = make_zip(&[("cudart64_12.dll", b"rt")]);
        let (e, srv) = engine(&[("main.zip", &main), ("rt.zip", &rt)]);
        let root = tmp_root("two");
        let dl = Downloader::new();
        let got = install(&dl, &root, &e, &e.builds[0], &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&got.exe).unwrap(), b"exe");
        assert!(got.dir.join("cudart64_12.dll").is_file());
        assert!(!root.join("downloads").join("main.zip").exists());
        assert_eq!(installed(&root, "llama.cpp").len(), 1);

        // Повторная установка ничего не качает.
        let before = srv[0].requests.load(Ordering::SeqCst);
        install(&dl, &root, &e, &e.builds[0], &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(srv[0].requests.load(Ordering::SeqCst), before);
    }

    #[tokio::test]
    async fn only_listed_files_are_unpacked() {
        let main = make_zip(&[("ff/bin/ffmpeg.exe", b"exe"), ("ff/bin/ffplay.exe", b"big"), ("ff/LICENSE", b"GPL")]);
        let (mut e, _srv) = engine(&[("ff.zip", &main)]);
        e.exe = "ff/bin/ffmpeg.exe".into();
        e.builds[0].files[0].only = vec!["ff/bin/ffmpeg.exe".into(), "ff/LICENSE".into()];
        let root = tmp_root("only");
        let got = install(&Downloader::new(), &root, &e, &e.builds[0], &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&got.exe).unwrap(), b"exe");
        assert!(got.dir.join("ff/LICENSE").is_file());
        assert!(!got.dir.join("ff/bin/ffplay.exe").exists());
    }

    #[test]
    fn tmp_keeps_whole_version() {
        let dir = Path::new(r"D:\Ollivo\engines\comfyui .37.0-cpu");
        assert_eq!(tmp_sibling(dir), Path::new(r"D:\Ollivo\engines\comfyui .37.0-cpu.tmp"));
        assert!(tmp_sibling(dir).extension().is_some_and(|e| e == "tmp"), "clean_leftovers ищет по расширению");
    }

    #[test]
    fn archive_paths_stay_inside() {
        let into = Path::new(r"D:\Ollivo\engines\x");
        assert_eq!(inside(into, "bin/ffmpeg.exe"), Some(into.join("bin").join("ffmpeg.exe")));
        for bad in ["../evil.exe", r"..\evil.exe", r"C:\Windows\evil.exe", "/etc/x", r"\\srv\share\x", ""] {
            assert_eq!(inside(into, bad), None, "{bad}");
        }
        assert!(wanted(&["a/b.exe".into()], r"a\b.exe") && !wanted(&["a/b.exe".into()], "a/c.exe"));
    }

    /// Настоящая сборка ffmpeg в 7z: распаковать только ffmpeg.exe.
    /// `OLLIVO_FF7Z=<путь к ffmpeg-9.0.2-essentials_build.7z> cargo test engines::tests::ffmpeg_7z_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ffmpeg_7z_real() {
        let archive = PathBuf::from(std::env::var("OLLIVO_FF7Z").unwrap());
        let e = crate::manifest::Manifest::bundled();
        let e = e.engine("ffmpeg").unwrap();
        let into = tmp_root("ff7z");
        let started = std::time::Instant::now();
        unpack_all(&[(archive, e.builds[0].files[0].only.clone())], &into).unwrap();
        println!("распаковано за {:.1} с", started.elapsed().as_secs_f64());
        let sums = file_sums(&into).unwrap();
        println!("{:#?}", sums.iter().map(|f| (&f.path, f.size)).collect::<Vec<_>>());
        let out = std::process::Command::new(into.join(&e.exe)).arg("-version").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("ffmpeg version 9.0.2"));
    }

    #[tokio::test]
    async fn archive_without_exe_leaves_nothing() {
        let main = make_zip(&[("readme.txt", b"hi")]);
        let (e, _srv) = engine(&[("main.zip", &main)]);
        let root = tmp_root("noexe");
        let err = install(&Downloader::new(), &root, &e, &e.builds[0], &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NoExe(_)));
        assert!(installed(&root, "llama.cpp").is_empty());
        assert!(!engine_dir(&root, &e, Build::Vulkan).exists());
    }

    #[tokio::test]
    async fn repair_finds_broken_file_and_reinstalls() {
        let main = make_zip(&[("llama-server.exe", b"exe"), ("lib/ggml.dll", b"dll")]);
        let (e, srv) = engine(&[("main.zip", &main)]);
        let root = tmp_root("repair");
        let dl = Downloader::new();
        let c = CancellationToken::new();
        let got = install(&dl, &root, &e, &e.builds[0], &c, &|_| {}).await.unwrap();

        // Всё цело — ничего не качаем.
        let before = srv[0].requests.load(Ordering::SeqCst);
        let r = repair(&dl, &root, &e, &e.builds[0], &c, &|_| {}).await.unwrap();
        assert!(r.broken.is_empty() && !r.reinstalled);
        assert_eq!(srv[0].requests.load(Ordering::SeqCst), before);

        // Антивирус «съел» библиотеку, а распаковка оборвалась в прошлый раз.
        std::fs::write(got.dir.join("lib").join("ggml.dll"), b"XXX").unwrap();
        let junk = tmp_sibling(&got.dir);
        std::fs::create_dir_all(&junk).unwrap();
        let r = repair(&dl, &root, &e, &e.builds[0], &c, &|_| {}).await.unwrap();
        assert_eq!(r.broken, vec!["lib/ggml.dll".to_string()]);
        assert!(r.reinstalled);
        assert_eq!(std::fs::read(got.dir.join("lib").join("ggml.dll")).unwrap(), b"dll");
        assert!(!junk.exists());
    }

    #[tokio::test]
    async fn repair_without_file_list_reinstalls() {
        let main = make_zip(&[("llama-server.exe", b"exe")]);
        let (e, _srv) = engine(&[("main.zip", &main)]);
        let root = tmp_root("repair-old");
        let dl = Downloader::new();
        let c = CancellationToken::new();
        let got = install(&dl, &root, &e, &e.builds[0], &c, &|_| {}).await.unwrap();
        std::fs::remove_file(got.dir.join(FILES)).unwrap();
        let r = repair(&dl, &root, &e, &e.builds[0], &c, &|_| {}).await.unwrap();
        assert!(r.reinstalled);
        assert!(got.dir.join(FILES).is_file());
    }

    /// Настоящий llama.cpp из встроенного манифеста: скачать, распаковать, запустить.
    /// `cargo test engines::tests::llama_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn llama_real() {
        let m = crate::manifest::Manifest::bundled();
        let e = m.engine("llama.cpp").unwrap();
        let b = e.pick(crate::hardware::detect().cuda_build, true, None).unwrap();
        let root = tmp_root("real");
        let started = std::time::Instant::now();
        let got = install(&Downloader::new(), &root, e, b, &CancellationToken::new(), &|_| {}).await.unwrap();
        println!("{:?} за {:.1} с", got.build, started.elapsed().as_secs_f64());
        let out = std::process::Command::new(&got.exe).arg("--version").output().unwrap();
        let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        println!("{text}");
        assert!(text.contains("11081"), "{text}");
    }

    /// Настоящий whisper.cpp из встроенного манифеста — в D:\Ollivo, где его ждут
    /// `speech::tests::real_transcribe` и программа.
    /// `cargo test engines::tests::whisper_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn whisper_real() {
        let m = crate::manifest::Manifest::bundled();
        let e = m.engine("whisper.cpp").unwrap();
        let b = e.pick(crate::hardware::detect().cuda_build, false, None).unwrap();
        let root = Path::new(r"D:\Ollivo");
        let started = std::time::Instant::now();
        let got = install(&Downloader::new(), root, e, b, &CancellationToken::new(), &|_| {}).await.unwrap();
        println!("{:?} за {:.1} с, {}", got.build, started.elapsed().as_secs_f64(), got.exe.display());
        let out = std::process::Command::new(&got.exe).arg("--help").output().unwrap();
        let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("supported audio formats"), "{text}");
    }
}
