#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    std::process::exit(nitor::win::main());

    #[cfg(not(windows))]
    {
        eprintln!("Nitor is a Windows application.");
        std::process::exit(1);
    }
}
