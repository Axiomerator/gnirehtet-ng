use relaylib::controller::ipc::{acquire_core_instance, serve};
use relaylib::controller::service::ControllerService;
use std::io::IsTerminal;

fn main() {
    let _instance = match acquire_core_instance() {
        Ok(instance) => instance,
        Err(error) => {
            eprintln!("Gnirehtet NG controller did not start: {error}");
            std::process::exit(2);
        }
    };
    let parent_pid = std::env::args()
        .skip_while(|argument| argument != "--parent-pid")
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok());
    let service = match ControllerService::start_with_parent(parent_pid) {
        Ok(service) => service,
        Err(fault) => {
            eprintln!("Gnirehtet NG controller did not start: {}", fault.message);
            std::process::exit(1);
        }
    };
    if std::io::stdout().is_terminal() {
        println!(
            "Gnirehtet NG controller PID {} listening on {}",
            std::process::id(),
            relaylib::controller::PIPE_NAME
        );
    }
    if let Err(error) = serve(service) {
        eprintln!("Gnirehtet NG controller stopped: {error}");
        std::process::exit(1);
    }
}
