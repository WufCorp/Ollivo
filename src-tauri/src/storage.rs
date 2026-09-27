//! Папка программы: сколько занято, уборка недокачанного, перенос на другой диск.
//!
//! Программе принадлежат только её подпапки (`PARTS`): в папку данных человек мог
//! положить и своё — это не считаем, не чистим и не переносим.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// Подпапки, которые ведёт программа.
pub const PARTS: [&str; 4] = ["models", "engines", "downloads", "logs"];

/// Недокачанное: загрузка пишет `<файл>.part` и рядом состояние `<файл>.part.json`.
fn is_partial(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.ends_with(".part") || name.ends_with(".part.json")
}

/// Сколько занято, по частям. Недокачанное внутри `models` считаем отдельно:
/// его можно удалить, а модели — нет.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Usage {
    pub models: u64,
    pub engines: u64,
    /// Недокачанное, архивы движков, временные записи для распознавания речи.
    pub cache: u64,
    pub logs: u64,
    /// Свободно на диске папки программы.
    pub free: u64,
}

impl Usage {
    /// Сколько переедет при переносе.
    pub fn total(&self) -> u64 {
        self.models + self.engines + self.cache + self.logs
    }
}

/// Обходит папку, вызывая `f` для каждого файла с его размером. Ссылки не раскрываем:
/// ссылка на чужую папку с моделями — не наши гигабайты.
fn walk(dir: &Path, f: &mut dyn FnMut(&Path, u64)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            walk(&e.path(), f);
        } else if t.is_file() {
            f(&e.path(), e.metadata().map_or(0, |m| m.len()));
        }
    }
}

/// Занятое место. Синхронная: на медленном диске обход тысяч файлов — секунды.
pub fn usage(root: &Path, free: u64) -> Usage {
    let mut u = Usage { free, ..Usage::default() };
    walk(&root.join("models"), &mut |p, n| if is_partial(p) { u.cache += n } else { u.models += n });
    walk(&root.join("engines"), &mut |p, n| {
        // Остатки оборванной установки (`<сборка>.tmp`) — мусор, а не движок.
        if p.ancestors().any(|a| a.extension().is_some_and(|e| e == "tmp")) {
            u.cache += n
        } else {
            u.engines += n
        }
    });
    walk(&root.join("downloads"), &mut |_, n| u.cache += n);
    walk(&root.join("logs"), &mut |_, n| u.logs += n);
    u
}

/// «Очистить кэш и недокачанное»: папка `downloads`, `.part` в моделях, остатки
/// оборванных установок движков. Сколько освободили. Звать, когда загрузок нет:
/// идущая загрузка пишет в тот самый `.part`.
pub fn clean(root: &Path) -> u64 {
    let mut freed = 0;
    let mut remove = |p: &Path, n: u64| {
        if std::fs::remove_file(p).is_ok() {
            freed += n;
        }
    };
    walk(&root.join("downloads"), &mut remove);
    walk(&root.join("models"), &mut |p, n| {
        if is_partial(p) {
            remove(p, n)
        }
    });
    if let Ok(ids) = std::fs::read_dir(root.join("engines")) {
        for id in ids.flatten() {
            let Ok(builds) = std::fs::read_dir(id.path()) else { continue };
            for b in builds.flatten() {
                if b.path().extension().is_some_and(|e| e == "tmp") {
                    let mut n = 0;
                    walk(&b.path(), &mut |_, s| n += s);
                    if std::fs::remove_dir_all(b.path()).is_ok() {
                        freed += n;
                    }
                }
            }
        }
    }
    freed
}

/// Куда переносить: выбранная папка, а если это не папка «Ollivo» — `<выбранная>\Ollivo`,
/// чтобы не рассыпать `models` и `engines` по корню диска или по «Документам».
pub fn target_dir(picked: &Path) -> PathBuf {
    if picked.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("ollivo")) {
        picked.to_path_buf()
    } else {
        picked.join("Ollivo")
    }
}

/// Можно ли переносить `old` → `new`: не в саму себя, не внутрь себя, и на новом месте
/// нет наших подпапок — сливать две папки программы мы не умеем.
pub fn check_target(old: &Path, new: &Path) -> Result<(), String> {
    let inside = |a: &Path, b: &Path| crate::library::strip_prefix_ci(a, b).is_some();
    if inside(new, old) || inside(old, new) {
        return Err(t!("новая папка не может быть внутри старой или содержать её", "the new folder can't be inside the old one or contain it").into());
    }
    if let Some(p) = PARTS.iter().map(|p| new.join(p)).find(|p| p.exists()) {
        return Err(format!("в новой папке уже есть «{}» — выберите пустую", p.file_name().unwrap().to_string_lossy()));
    }
    Ok(())
}

/// Один диск или разные: на одном перенос — переименование, мгновенно.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    root(a).is_some() && root(a) == root(b)
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// Переносит наши подпапки. Сначала всё копируется, и только потом старое удаляется:
/// оборвалось посередине (место кончилось, «Отменить») — копии убираются, старая папка
/// цела и программа работает как раньше. На том же диске — просто переименование.
pub fn move_parts(
    old: &Path,
    new: &Path,
    cancel: &CancellationToken,
    progress: &dyn Fn(Progress),
) -> Result<(), String> {
    transfer(old, new, cancel, progress, same_volume(old, new))
}

/// `rename` — переименованием (тот же диск), иначе копированием.
fn transfer(
    old: &Path,
    new: &Path,
    cancel: &CancellationToken,
    progress: &dyn Fn(Progress),
    rename: bool,
) -> Result<(), String> {
    std::fs::create_dir_all(new).map_err(|e| tf!("не создать {}: {e}", "could not create {}: {e}", new.display()))?;
    let parts: Vec<&str> = PARTS.iter().copied().filter(|p| old.join(p).exists()).collect();
    if rename {
        let mut done: Vec<&str> = vec![];
        for p in &parts {
            if let Err(e) = std::fs::rename(old.join(p), new.join(p)) {
                // Вернуть то, что уже переехало, — иначе программа окажется в двух папках.
                for d in &done {
                    let _ = std::fs::rename(new.join(d), old.join(d));
                }
                return Err(tf!(
                    "не перенести «{p}»: {e}. Возможно, файл открыт другой программой.",
                    "could not move “{p}”: {e}. The file may be open in another program."
                ));
            }
            done.push(p);
        }
        return Ok(());
    }
    let mut total = 0;
    for p in &parts {
        walk(&old.join(p), &mut |_, n| total += n);
    }
    let mut done = 0;
    let res = (|| {
        for p in &parts {
            copy_tree(&old.join(p), &new.join(p), cancel, &mut |n| {
                done += n;
                progress(Progress { done, total });
            })?;
        }
        Ok(())
    })();
    if let Err(e) = res {
        for p in &parts {
            let _ = std::fs::remove_dir_all(new.join(p));
        }
        let _ = std::fs::remove_dir(new);
        return Err(e);
    }
    // Старое — только после того, как всё скопировано. Не удалилось (файл занят) —
    // не беда: программа уже работает из новой папки, место освободится позже вручную.
    for p in &parts {
        let _ = std::fs::remove_dir_all(old.join(p));
    }
    Ok(())
}

/// Копирует папку кусками по 8 МБ: прогресс идёт и внутри файла на 20 ГБ,
/// а «Отменить» срабатывает сразу.
fn copy_tree(from: &Path, to: &Path, cancel: &CancellationToken, step: &mut dyn FnMut(u64)) -> Result<(), String> {
    use std::io::{Read, Write};
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type().map_err(|e| e.to_string())?;
        if t.is_dir() {
            copy_tree(&src, &dst, cancel, step)?;
            continue;
        }
        if !t.is_file() {
            continue;
        }
        let mut r = std::fs::File::open(&src).map_err(|e| tf!("не открыть {}: {e}", "could not open {}: {e}", src.display()))?;
        let mut w = std::fs::File::create(&dst).map_err(|e| tf!("не записать {}: {e}", "could not write {}: {e}", dst.display()))?;
        let mut buf = vec![0u8; 8 << 20];
        loop {
            if cancel.is_cancelled() {
                return Err(crate::llm::CANCELLED.into());
            }
            let n = r.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|e| tf!("не записать {}: {e}", "could not write {}: {e}", dst.display()))?;
            step(n as u64);
        }
        // Дата изменения нужна библиотеке: по ней она понимает, что файл тот же
        // и заголовок модели перечитывать не надо.
        if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
            let _ = w.set_modified(m);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = crate::testserver::tmp().join(format!("ollivo-storage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, size: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; size]).unwrap();
    }

    /// Папка как у программы: модель, недокачанное, движок, остаток установки, архив, журнал
    /// и чужой файл человека, который нас не касается.
    fn sample(root: &Path) {
        put(&root.join(r"models\a\b\m.gguf"), 1000);
        put(&root.join(r"models\a\b\n.gguf.part"), 300);
        put(&root.join(r"models\a\b\n.gguf.part.json"), 10);
        put(&root.join(r"engines\llama.cpp\b1-vulkan\llama-server.exe"), 500);
        put(&root.join(r"engines\llama.cpp\b2-vulkan.tmp\llama-server.exe"), 50);
        put(&root.join(r"downloads\x.zip"), 200);
        put(&root.join(r"logs\llama.log"), 40);
        put(&root.join(r"мои заметки.txt"), 5);
    }

    #[test]
    fn usage_and_clean() {
        let root = tmp("usage");
        sample(&root);
        let u = usage(&root, 9);
        assert_eq!(u, Usage { models: 1000, engines: 500, cache: 560, logs: 40, free: 9 });
        assert_eq!(clean(&root), 560);
        let u = usage(&root, 9);
        assert_eq!((u.models, u.engines, u.cache, u.logs), (1000, 500, 0, 40));
        assert!(root.join(r"models\a\b\m.gguf").exists());
        assert!(root.join("мои заметки.txt").exists());
    }

    #[test]
    fn target_and_checks() {
        assert_eq!(target_dir(Path::new(r"E:\")), PathBuf::from(r"E:\Ollivo"));
        assert_eq!(target_dir(Path::new(r"E:\Games\ollivo")), PathBuf::from(r"E:\Games\ollivo"));
        assert!(check_target(Path::new(r"D:\Ollivo"), Path::new(r"D:\Ollivo\new")).is_err());
        assert!(check_target(Path::new(r"D:\Ollivo\x"), Path::new(r"D:\Ollivo")).is_err());
        assert!(same_volume(Path::new(r"D:\Ollivo"), Path::new(r"d:\Other")));
        assert!(!same_volume(Path::new(r"D:\Ollivo"), Path::new(r"E:\Ollivo")));
    }

    /// Копирование между папками (как между дисками): всё на новом месте, старого нет,
    /// чужой файл человека остался где был.
    #[test]
    fn copy_move_and_cancel() {
        let base = tmp("move");
        let (old, new) = (base.join("old"), base.join("new"));
        sample(&old);
        assert!(check_target(&old, &new).is_ok());

        // «Отменить» посередине — копии убраны, старое цело.
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(transfer(&old, &new, &cancel, &|_| {}, false).is_err());
        assert!(!new.join("models").exists());
        assert!(old.join(r"models\a\b\m.gguf").exists());

        let last = std::cell::Cell::new(Progress { done: 0, total: 0 });
        let cancel = CancellationToken::new();
        transfer(&old, &new, &cancel, &|p| last.set(p), false).unwrap();
        assert_eq!(std::fs::metadata(new.join(r"models\a\b\m.gguf")).unwrap().len(), 1000);
        assert!(new.join(r"engines\llama.cpp\b1-vulkan\llama-server.exe").exists());
        assert!(!old.join("models").exists());
        assert!(old.join("мои заметки.txt").exists());
        assert_eq!(last.get().done, last.get().total);
        assert_eq!(last.get().total, 1000 + 310 + 550 + 200 + 40);

        // Туда, где уже есть наша папка, — нельзя.
        assert!(check_target(&old, &new).is_err());

        // На том же диске — переименованием, туда и обратно.
        let back = base.join("back");
        transfer(&new, &back, &cancel, &|_| {}, true).unwrap();
        assert!(back.join(r"models\a\b\m.gguf").exists() && !new.join("models").exists());
    }
}
