#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    flectar_mail::run(flectar_mail::PlatformContext::desktop()?)
}
