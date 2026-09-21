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
