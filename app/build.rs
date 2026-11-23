//! Compila el contexto de Tauri (iconos, permisos, ventanas) y el manifiesto.
//!
//! El manifiesto fija `asInvoker`, DPI por monitor y rutas largas. Tauri añade
//! además icono, versión y nombre de producto al `.exe` desde `tauri.conf.json`.

fn main() {
    let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attributes).expect("tauri-build");
}
