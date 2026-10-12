#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main_entry();

    #[cfg(not(windows))]
    println!("Windows only");
}
