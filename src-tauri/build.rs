use std::path::{Path, PathBuf};

/// Библиотеки VC++, которые импортируют llama.cpp и whisper.cpp. Программа кладёт их
/// рядом с движком (`vcrt.rs`), чтобы на чистой Windows не ставить VC++ через UAC.
const VC_DLLS: &[&str] = &["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

fn main() {
    vc_runtime();
    tauri_build::build()
}

/// Копирует DLL из Redist установленной Visual Studio в `OUT_DIR/vcrt`: оттуда их
/// вшивает `include_bytes!`. Без Visual Studio Rust под MSVC не собирается вовсе,
/// так что Redist есть везде, где идёт сборка; `OLLIVO_VCRT_DIR` — если он в другом месте.
fn vc_runtime() {
    println!("cargo:rerun-if-env-changed=OLLIVO_VCRT_DIR");
    println!("cargo:rerun-if-env-changed=VCToolsRedistDir");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("vcrt");
    std::fs::create_dir_all(&out).unwrap();
    let Some(src) = find_crt() else {
        panic!(
            "не нашёл библиотеки VC++ (msvcp140.dll и др.) в Redist Visual Studio. \
             Поставьте компонент «C++ Redistributable» в Visual Studio Installer \
             или укажите папку с ними в OLLIVO_VCRT_DIR"
        );
    };
    for dll in VC_DLLS {
        std::fs::copy(src.join(dll), out.join(dll)).unwrap_or_else(|e| panic!("{}: {e}", src.join(dll).display()));
    }
}

fn find_crt() -> Option<PathBuf> {
    let has_all = |d: &Path| VC_DLLS.iter().all(|f| d.join(f).is_file());
    if let Ok(d) = std::env::var("OLLIVO_VCRT_DIR") {
        return Some(PathBuf::from(d)).filter(|d| has_all(d));
    }
    // Командная строка разработчика VS задаёт путь к Redist сама.
    let mut redist_roots: Vec<PathBuf> = std::env::var("VCToolsRedistDir").ok().map(PathBuf::from).into_iter().collect();
    let vswhere = Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if let Ok(o) = std::process::Command::new(vswhere)
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
        .output()
    {
        let vs = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !vs.is_empty() {
            // VC\Redist\MSVC\<версия>\x64\Microsoft.VC143.CRT — берём самую новую версию.
            let msvc = Path::new(&vs).join(r"VC\Redist\MSVC");
            let mut vers: Vec<PathBuf> = std::fs::read_dir(&msvc).into_iter().flatten().flatten().map(|e| e.path()).collect();
            vers.sort_by_key(|p| version_key(p));
            redist_roots.extend(vers.into_iter().rev());
        }
    }
    redist_roots.iter().find_map(|root| {
        let x64 = root.join("x64");
        let mut crts: Vec<PathBuf> = std::fs::read_dir(&x64)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".CRT")))
            .collect();
        crts.sort();
        crts.into_iter().rev().find(|d| has_all(d))
    })
}

/// «14.44.35112» → [14, 44, 35112]; папки вроде «v143» (там только модули слияния) — в самый низ.
fn version_key(p: &Path) -> Vec<u32> {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    name.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}
