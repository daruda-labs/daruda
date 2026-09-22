use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut exit = 0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdout" => {
                print!("{}", args.next().expect("stdout text"));
                std::io::stdout().flush().unwrap();
            }
            "--stderr" => {
                std::io::stderr()
                    .write_all(args.next().expect("stderr text").as_bytes())
                    .unwrap();
            }
            "--sleep-ms" => std::thread::sleep(std::time::Duration::from_millis(
                args.next().expect("milliseconds").parse().unwrap(),
            )),
            "--exit" => exit = args.next().expect("exit code").parse().unwrap(),
            "--require-env" => {
                let name = args.next().expect("environment name");
                let value = args.next().expect("environment value");
                if std::env::var(name).ok().as_deref() != Some(&value) {
                    exit = 1;
                }
            }
            "--reject-env-value" => {
                let name = args.next().expect("environment name");
                let value = args.next().expect("environment value");
                if std::env::var(name).ok().as_deref() == Some(&value) {
                    exit = 1;
                }
            }
            "--flood" => {
                let mut stdout = std::io::stdout().lock();
                while stdout.write_all(&[b'x'; 8192]).is_ok() {}
            }
            // Fork a copy of this fixture that outlives us, writing its pid
            // where the test can read it. What a tree-kill has to reach and a
            // `child.kill()` never does.
            "--orphan" => {
                let pid_file = args.next().expect("pid file");
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    // Detached from our streams and short-lived on its own:
                    // a test that fails to kill it must not then hand the
                    // harness a pipe nobody will close.
                    .args(["--sleep-ms", "60000"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("spawn grandchild");
                std::fs::write(&pid_file, child.id().to_string()).expect("write pid");
            }
            "--absent-env" => {
                if std::env::var_os(args.next().expect("environment name")).is_some() {
                    exit = 1;
                }
            }
            _ => panic!("unknown fixture argument: {arg}"),
        }
    }
    std::process::exit(exit);
}
