//! Установка движков по манифесту: скачать архивы, проверить, распаковать.
//!
//! Раскладка в папке данных (`<диск>:\Ollivo`):
//! ```text
//! downloads\<архив>.zip                    — пока идёт установка, потом удаляется
//! engines\<id>\<версия>-<сборка>\          — готовый движок
//! engines\<id>\<версия>-<сборка>\ollivo.json — метка «установлено полностью»
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
        let json = std::fs::read(d.ok()?.path().join(MARKER)).ok()?;
        serde_json::from_slice::<Installed>(&json).ok()
    })
    .filter(|i| i.exe.exists())
    .collect()
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
        archives.push(req.dest);
    }

    on_progress(InstallProgress { stage: Stage::Unpack, done: total, total, speed: 0.0 });
    let tmp = dir.with_extension("tmp");
    let (tmp2, archives2) = (tmp.clone(), archives.clone());
    tokio::task::spawn_blocking(move || unpack_all(&archives2, &tmp2))
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
    std::fs::write(tmp.join(MARKER), serde_json::to_vec_pretty(&done).unwrap())?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&tmp, &dir)?;
    for a in &archives {
        let _ = std::fs::remove_file(a);
    }
    Ok(done)
}

fn unpack_all(archives: &[PathBuf], into: &Path) -> Result<(), Error> {
    if into.exists() {
        std::fs::remove_dir_all(into)?;
    }
    std::fs::create_dir_all(into)?;
    for a in archives {
        let name = a.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let bad = |e: &dyn std::fmt::Display| Error::Unpack(name.clone(), e.to_string());
        let mut zip = zip::ZipArchive::new(std::fs::File::open(a)?).map_err(|e| bad(&e))?;
        // `extract` сам отбрасывает пути вида `..\..\` (zip slip).
        zip.extract(into).map_err(|e| bad(&e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let dir = std::env::temp_dir().join(format!("ollivo-eng-{}-{name}", std::process::id()));
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
}
