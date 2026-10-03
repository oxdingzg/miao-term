//! Install a verified update and relaunch (B4.3, ADR 0025).
//!
//! A running app cannot replace itself, so a small detached helper waits for
//! our process to exit, swaps the app and starts it again:
//!
//! - macOS: the `.app` from the zip replaces the running bundle; the old one
//!   is moved aside first and put back if the move fails.
//! - Linux: a running AppImage (`$APPIMAGE`) is overwritten. Other formats
//!   (deb, tarball) belong to the package manager.
//! - Windows: an MSI runs with `msiexec`; a zip is copied over the install.
//!
//! Only artifacts that passed [`crate::update::download_verified`] get here.

use std::path::{Path, PathBuf};

/// A path as one `sh` word.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// The `.app` bundle containing `exe` (`…/X.app/Contents/MacOS/x` → `…/X.app`).
pub fn bundle_root(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    (bundle.extension().and_then(|e| e.to_str()) == Some("app")).then(|| bundle.to_path_buf())
}

/// The app bundle in an unpacked archive: `mtty.app`, or `miaotty.app` from a
/// release made before the rename.
pub fn find_app(dir: &Path) -> Option<PathBuf> {
    for wanted in ["mtty.app", "miaotty.app"] {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.file_name().and_then(|n| n.to_str()) == Some(wanted) {
                    return Some(path);
                }
                if path.is_dir() && path.extension().and_then(|e| e.to_str()) != Some("app") {
                    stack.push(path);
                }
            }
        }
    }
    None
}

/// macOS: swap `bundle` for `new_app` once `pid` exits, then relaunch;
/// restore the old bundle if the swap fails.
pub fn macos_script(pid: u32, bundle: &Path, new_app: &Path) -> String {
    let old = bundle.with_file_name(format!(
        "{}.old",
        bundle.file_name().unwrap_or_default().to_string_lossy()
    ));
    let (bundle, new_app, old) = (sh_quote(bundle), sh_quote(new_app), sh_quote(&old));
    format!(
        "#!/bin/sh\n\
         # mtty update helper\n\
         while kill -0 {pid} 2>/dev/null; do sleep 0.3; done\n\
         rm -rf {old}\n\
         mv {bundle} {old} || exit 1\n\
         if mv {new_app} {bundle}; then\n\
         \x20 rm -rf {old}\n\
         \x20 open {bundle}\n\
         else\n\
         \x20 mv {old} {bundle}\n\
         \x20 open {bundle}\n\
         fi\n"
    )
}

/// Linux: overwrite the running AppImage once `pid` exits and start it.
pub fn appimage_script(pid: u32, appimage: &Path, downloaded: &Path) -> String {
    let (appimage, downloaded) = (sh_quote(appimage), sh_quote(downloaded));
    format!(
        "#!/bin/sh\n\
         while kill -0 {pid} 2>/dev/null; do sleep 0.3; done\n\
         tmp={appimage}.new\n\
         cp {downloaded} \"$tmp\" && chmod +x \"$tmp\" && mv \"$tmp\" {appimage}\n\
         exec {appimage}\n"
    )
}

/// Windows: a `.cmd` that waits for `pid`, then runs the MSI (or unpacks a
/// zip over the install directory) and starts the app again.
pub fn windows_script(pid: u32, artifact: &Path, exe: &Path) -> String {
    let artifact = artifact.display();
    // Split on either separator: the script is also built (and tested) off
    // Windows, where `Path` does not know `\\`.
    let exe_text = exe.to_string_lossy();
    let dir = exe_text
        .rfind(['\\', '/'])
        .map(|i| exe_text[..i].to_string())
        .unwrap_or_default();
    let exe = exe.display();
    format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         echo \"{artifact}\" | find /I \".msi\" >nul\r\n\
         if not errorlevel 1 (\r\n\
         \x20 msiexec /i \"{artifact}\" /passive /norestart\r\n\
         \x20 if exist \"%ProgramFiles%\\mtty\\bin\\mtty.exe\" (start \"\" \"%ProgramFiles%\\mtty\\bin\\mtty.exe\") else (start \"\" \"{exe}\")\r\n\
         ) else (\r\n\
         \x20 rmdir /s /q \"%TEMP%\\mtty-update\" 2>nul\r\n\
         \x20 mkdir \"%TEMP%\\mtty-update\"\r\n\
         \x20 tar -xf \"{artifact}\" -C \"%TEMP%\\mtty-update\"\r\n\
         \x20 for /r \"%TEMP%\\mtty-update\" %%f in (mtty.exe mtty-cli.exe mtty-ptyhost.exe) do copy /y \"%%f\" \"{dir}\\%%~nxf\" >nul\r\n\
         \x20 start \"\" \"{exe}\"\r\n\
         )\r\n"
    )
}

/// What installing needs from the caller.
pub enum Plan {
    /// A helper is staged: start it with [`launch`], then quit.
    Helper(PathBuf),
    /// This install cannot replace itself (a dev build, a deb, a tarball):
    /// open the verified download instead.
    OpenDownload(String),
}

/// Prepare the swap for a verified artifact. Unpacks on macOS, so it blocks
/// for a moment.
pub fn prepare(artifact: &Path) -> Result<Plan, String> {
    let pid = std::process::id();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stage = std::env::temp_dir().join(format!("mtty-update-{pid}"));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let _ = &exe;
    #[cfg(target_os = "macos")]
    {
        let Some(bundle) = bundle_root(&exe) else {
            return Ok(Plan::OpenDownload(
                "not running from an app bundle (a development build)".into(),
            ));
        };
        let unzip = std::process::Command::new("ditto")
            .args(["-x", "-k"])
            .arg(artifact)
            .arg(&stage)
            .status()
            .map_err(|e| e.to_string())?;
        if !unzip.success() {
            return Err("could not unpack the update".into());
        }
        let new_app = find_app(&stage).ok_or("no mtty.app in the update")?;
        let script = stage.join("install.sh");
        std::fs::write(&script, macos_script(pid, &bundle, &new_app)).map_err(|e| e.to_string())?;
        Ok(Plan::Helper(script))
    }
    #[cfg(target_os = "linux")]
    {
        let is_appimage = artifact
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("AppImage"));
        match std::env::var_os("APPIMAGE") {
            Some(running) if is_appimage => {
                let script = stage.join("install.sh");
                std::fs::write(&script, appimage_script(pid, Path::new(&running), artifact))
                    .map_err(|e| e.to_string())?;
                Ok(Plan::Helper(script))
            }
            _ => Ok(Plan::OpenDownload(
                "installed by a package manager; install the download with it".into(),
            )),
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = stage.join("install.cmd");
        std::fs::write(&script, windows_script(pid, artifact, &exe)).map_err(|e| e.to_string())?;
        Ok(Plan::Helper(script))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = artifact;
        Ok(Plan::OpenDownload("no installer for this platform".into()))
    }
}

/// Start the running app again once `pid` exits: a relaunch that keeps the
/// panes' programs running in their PTY hosts (ADR 0041). The bundle on
/// macOS, the AppImage when running from one, else the executable itself.
pub fn relaunch_script(pid: u32, exe: &Path, appimage: Option<&Path>) -> String {
    let start = match (bundle_root(exe), appimage) {
        (Some(bundle), _) => format!("open {}", sh_quote(&bundle)),
        (None, Some(image)) => format!("{} >/dev/null 2>&1 &", sh_quote(image)),
        (None, None) => format!("{} >/dev/null 2>&1 &", sh_quote(exe)),
    };
    format!("#!/bin/sh\nwhile kill -0 {pid} 2>/dev/null; do sleep 0.1; done\n{start}\n")
}

/// Windows: start `exe` again once `pid` exits (see [`relaunch_script`]).
pub fn windows_relaunch_script(pid: u32, exe: &Path) -> String {
    format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         start \"\" \"{}\"\r\n",
        exe.display()
    )
}

/// Stage and start the relaunch helper; the caller quits right after.
pub fn relaunch() -> Result<(), String> {
    let pid = std::process::id();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stage = std::env::temp_dir().join(format!("mtty-relaunch-{pid}"));
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let (script, text) = if cfg!(windows) {
        (
            stage.join("relaunch.cmd"),
            windows_relaunch_script(pid, &exe),
        )
    } else {
        let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
        (
            stage.join("relaunch.sh"),
            relaunch_script(pid, &exe, appimage.as_deref()),
        )
    };
    std::fs::write(&script, text).map_err(|e| e.to_string())?;
    launch(&script).map_err(|e| e.to_string())
}

/// QA settings that run once per launch (`AGENTS.md`), never again in the
/// app a helper starts.
const ONE_LAUNCH_ENV: &[&str] = &["MTTY_QA_COMMAND", "MTTY_QA_AFTER", "MTTY_QA_SCROLL"];

/// Start a staged helper detached from us, so it outlives our exit.
pub fn launch(script: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = std::process::Command::new("cmd");
        for name in ONE_LAUNCH_ENV {
            cmd.env_remove(name);
        }
        cmd.args(["/c", "start", "", "/b"])
            .arg(script)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let detach = if cfg!(target_os = "linux") {
            "setsid "
        } else {
            ""
        };
        let mut cmd = std::process::Command::new("sh");
        for name in ONE_LAUNCH_ENV {
            cmd.env_remove(name);
        }
        cmd.arg("-c")
            .arg(format!(
                "{detach}nohup sh {} >/dev/null 2>&1 &",
                sh_quote(script)
            ))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_relaunch_waits_then_starts_the_exe() {
        let script = windows_relaunch_script(7, Path::new(r"C:\Program Files\mtty\bin\mtty.exe"));
        assert!(script.contains("PID eq 7"), "{script}");
        assert!(
            script.ends_with("start \"\" \"C:\\Program Files\\mtty\\bin\\mtty.exe\"\r\n"),
            "{script}"
        );
    }

    #[test]
    fn relaunch_starts_what_is_running_once_it_has_quit() {
        let wait = "while kill -0 42 2>/dev/null; do sleep 0.1; done\n";
        let bundle = relaunch_script(
            42,
            Path::new("/Applications/mtty.app/Contents/MacOS/mtty"),
            None,
        );
        assert!(bundle.contains(wait), "{bundle}");
        assert!(
            bundle.ends_with("open '/Applications/mtty.app'\n"),
            "{bundle}"
        );
        let image = relaunch_script(
            42,
            Path::new("/tmp/.mount_x/usr/bin/mtty"),
            Some(Path::new("/opt/apps/mtty.AppImage")),
        );
        assert!(
            image.ends_with("'/opt/apps/mtty.AppImage' >/dev/null 2>&1 &\n"),
            "{image}"
        );
        let plain = relaunch_script(42, Path::new("/opt/it's/mtty"), None);
        assert!(
            plain.ends_with("'/opt/it'\\''s/mtty' >/dev/null 2>&1 &\n"),
            "{plain}"
        );
    }

    #[test]
    fn bundles_and_apps_are_found() {
        let exe = Path::new("/Applications/mtty.app/Contents/MacOS/mtty");
        assert_eq!(
            bundle_root(exe).as_deref(),
            Some(Path::new("/Applications/mtty.app"))
        );
        assert!(bundle_root(Path::new("/tmp/target/release/mtty")).is_none());
        let root = std::env::temp_dir().join(format!("mtty-find-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("x/miaotty.app/Contents")).unwrap();
        assert_eq!(find_app(&root), Some(root.join("x/miaotty.app")));
        std::fs::create_dir_all(root.join("mtty.app/Contents")).unwrap();
        assert_eq!(
            find_app(&root),
            Some(root.join("mtty.app")),
            "the new name wins"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_and_linux_scripts_wait_then_relaunch() {
        let win = windows_script(
            4242,
            Path::new(r"C:\Users\me\Downloads\mtty-0.1.0-x86_64.msi"),
            Path::new(r"C:\Program Files\mtty\bin\mtty.exe"),
        );
        assert!(win.contains("PID eq 4242") && win.contains("msiexec /i"));
        assert!(win.contains(r"C:\Program Files\mtty\bin\%%~nxf"), "{win}");
        let lin = appimage_script(
            4242,
            Path::new("/opt/it's mtty.AppImage"),
            Path::new("/tmp/d"),
        );
        assert!(lin.contains("kill -0 4242"));
        assert!(lin.contains(r"exec '/opt/it'\''s mtty.AppImage'"), "{lin}");
    }

    /// The real helpers run under sh: the macOS swap (and its rollback) and
    /// the AppImage replacement, with `open` and the app faked.
    #[test]
    #[cfg(unix)]
    fn helpers_swap_and_roll_back() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("mtty-helper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let open = bin.join("open");
        std::fs::write(&open, "#!/bin/sh\nprintf '%s' \"$1\" > \"$OPEN_LOG\"\n").unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        for succeeds in [true, false] {
            let case = root.join(if succeeds { "ok" } else { "fail" });
            let installed = case.join("Apps $HOME's/mtty.app");
            let downloaded = case.join("new/mtty.app");
            std::fs::create_dir_all(&installed).unwrap();
            std::fs::write(installed.join("version"), "old").unwrap();
            if succeeds {
                std::fs::create_dir_all(&downloaded).unwrap();
                std::fs::write(downloaded.join("version"), "new").unwrap();
            }
            let log = case.join("open.log");
            // u32::MAX is never a live pid, so the helper does not wait.
            let out = std::process::Command::new("sh")
                .args(["-c", &macos_script(u32::MAX, &installed, &downloaded)])
                .env("PATH", &path)
                .env("OPEN_LOG", &log)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let version = std::fs::read_to_string(installed.join("version")).unwrap();
            assert_eq!(version, if succeeds { "new" } else { "old" });
            assert!(!installed.with_file_name("mtty.app.old").exists());
            assert_eq!(
                std::fs::read_to_string(&log).unwrap(),
                installed.to_string_lossy()
            );
        }
        // AppImage: the running file is replaced and started.
        let app = root.join("my apps/mtty.AppImage");
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::write(&app, "#!/bin/sh\necho old\n").unwrap();
        let new = root.join("dl.AppImage");
        std::fs::write(&new, "#!/bin/sh\necho started new\n").unwrap();
        let out = std::process::Command::new("sh")
            .args(["-c", &appimage_script(u32::MAX, &app, &new)])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "started new");
        assert!(std::fs::metadata(&app).unwrap().permissions().mode() & 0o111 != 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
