mod act;
mod changed;
mod config;
mod filter;
mod group;
mod help;
mod hidden;
mod index;
mod load;
mod output;
mod pattern;
mod piped;
mod rank;
mod search;
mod setup;
mod stop;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    home: bool,
}

impl Fixture {
    fn new() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "syk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let fixture = Fixture { root, home: false };
        fixture.dir(".git");
        fixture.file(".gitignore", "ignored/\n");
        fixture.file(".hidden/secret.txt", "needle hidden\n");
        fixture.file("ignored/skip.txt", "needle ignored\n");
        fixture.file(
            "src/api/auth.ts",
            "export function login() {}\nconst needle = 1\n",
        );
        fixture.file("src/api/route.ts", "needle in route\nother\nlast\n");
        fixture.file("src/web/route.tsx", "nothing here\n");
        fixture.file("src/web/needle.md", "title\n");
        fixture.file("needle/keep.txt", "plain\n");
        fixture.file("big.bin", "needle\0binary\n");
        fixture.file("notes.log", &"x".repeat(5000));
        fixture
    }

    fn data(&self) -> PathBuf {
        self.root.with_extension("data")
    }

    fn dir(&self, path: &str) {
        fs::create_dir_all(self.root.join(path)).expect("create folder");
    }

    fn file(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).expect("create parent");
        fs::write(path, text).expect("write file");
    }

    fn command(&self, args: &[&str]) -> Command {
        let home = self.data();
        let mut command = Command::new(env!("CARGO_BIN_EXE_syk"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("NO_COLOR", "1")
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CONFIG_HOME", self.data())
            .env("XDG_CACHE_HOME", self.data())
            .env("HOME", if self.home { &self.root } else { &home })
            .env("_ZO_DATA_DIR", self.data())
            .env_remove("SYK_CONFIG")
            .env_remove("VISUAL")
            .env_remove("EDITOR");
        command
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let output = self.command(args).output().expect("run syk");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.code().unwrap_or(-1),
        )
    }

    fn errors(&self, args: &[&str]) -> String {
        let output = self.command(args).output().expect("run syk");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn outside(&self, path: &str, text: &str) -> PathBuf {
        let path = self.data().join(path);
        fs::create_dir_all(path.parent().expect("a parent folder")).expect("create parent");
        fs::write(&path, text).expect("write file");
        path
    }

    fn lines(&self, args: &[&str]) -> Vec<String> {
        let mut args = args.to_vec();
        args.push("-o");
        self.run(&args).0.lines().map(str::to_string).collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(self.data());
    }
}

#[cfg(target_os = "linux")]
fn forget(path: &Path) {
    use std::os::fd::AsRawFd;

    let file = File::open(path).expect("open file");
    file.sync_all().expect("write file to disk");
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

#[cfg(not(target_os = "linux"))]
fn forget(_: &Path) {}

const PATIENCE: Duration = Duration::from_secs(10);
const PAUSE: Duration = Duration::from_millis(20);

struct Terminal {
    screen: File,
    child: Child,
    seen: String,
    read: usize,
}

impl Terminal {
    fn open(fixture: &Fixture, args: &[&str], piped: bool) -> Terminal {
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut screen, mut inside) = (0, 0);
        let opened = unsafe {
            libc::openpty(
                &mut screen,
                &mut inside,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "no pseudo-terminal");
        let sized = unsafe { libc::ioctl(screen, libc::TIOCSWINSZ, &size) };
        assert_eq!(sized, 0, "no window size");
        let flags = unsafe { libc::fcntl(screen, libc::F_GETFL) };
        let free = unsafe { libc::fcntl(screen, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        assert_eq!(free, 0, "the terminal cannot be read without waiting");
        let inside = unsafe { File::from_raw_fd(inside) };
        let mut command = fixture.command(args);
        command
            .env("TERM", "xterm-256color")
            .env_remove("SYK_PAGER")
            .stdin(inside.try_clone().expect("copy the terminal"))
            .stderr(inside.try_clone().expect("copy the terminal"));
        match piped {
            true => command.stdout(Stdio::piped()),
            false => command.stdout(inside.try_clone().expect("copy the terminal")),
        };
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("start syk");
        Terminal {
            screen: unsafe { File::from_raw_fd(screen) },
            child,
            seen: String::new(),
            read: 0,
        }
    }

    fn wait_for(&mut self, text: &str) {
        let started = Instant::now();
        loop {
            if let Some(at) = self.seen[self.read..].find(text) {
                self.read += at + text.len();
                return;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "{text:?} never showed up after {:?}",
                self.seen[self.read..]
                    .chars()
                    .rev()
                    .take(300)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
            let mut chunk = [0u8; 8192];
            match self.screen.read(&mut chunk) {
                Ok(read) if read > 0 => {
                    self.seen.push_str(&String::from_utf8_lossy(&chunk[..read]));
                }
                _ => std::thread::sleep(PAUSE),
            }
        }
    }

    fn press(&mut self, keys: &str, then: &str) {
        self.screen.write_all(keys.as_bytes()).expect("send keys");
        self.wait_for(then);
    }

    fn shown(mut self) -> (String, i32) {
        let started = Instant::now();
        let mut ended = None;
        loop {
            let mut chunk = [0u8; 8192];
            match self.screen.read(&mut chunk) {
                Ok(read) if read > 0 => {
                    self.seen.push_str(&String::from_utf8_lossy(&chunk[..read]));
                }
                _ if ended.is_some() => break,
                _ => {
                    ended = self.child.try_wait().expect("wait for syk");
                    assert!(started.elapsed() < PATIENCE, "syk did not exit");
                    std::thread::sleep(PAUSE);
                }
            }
        }
        let status = ended.and_then(|status| status.code()).unwrap_or(-1);
        (std::mem::take(&mut self.seen), status)
    }

    fn finish(mut self, keys: &str) -> (String, i32) {
        self.screen.write_all(keys.as_bytes()).expect("send keys");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for syk") {
                break status;
            }
            if started.elapsed() > PATIENCE {
                let _ = self.child.kill();
                panic!("syk did not exit after {keys:?}");
            }
            let mut chunk = [0u8; 8192];
            if !matches!(self.screen.read(&mut chunk), Ok(read) if read > 0) {
                std::thread::sleep(PAUSE);
            }
        };
        let mut printed = String::new();
        if let Some(mut stdout) = self.child.stdout.take() {
            stdout
                .read_to_string(&mut printed)
                .expect("read the output");
        }
        (printed, status.code().unwrap_or(-1))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
