//! Puente mínimo con Win32: consola de la CLI y aviso de WebView2 ausente.
//!
//! Es el único sitio del workspace con `unsafe`. Todo lo demás compila con
//! `unsafe_code = "deny"` (el `allow` de este módulo es local y explícito).

#![allow(unsafe_code)]

/// En Windows, engancha la salida al padre cuando el `.exe` se lanza desde una
/// consola (PowerShell, cmd). En otros sistemas no hay nada que hacer.
#[cfg(not(windows))]
pub fn adjuntar_consola() {}

/// En Windows, comprueba WebView2 y, si falta, muestra un aviso nativo con el
/// enlace de descarga. Devuelve `false` cuando la app no puede continuar.
#[cfg(not(windows))]
#[must_use]
pub fn webview2_listo() -> bool {
    true
}

#[cfg(windows)]
pub use windows_impl::{adjuntar_consola, webview2_listo};

#[cfg(windows)]
mod windows_impl {
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };

    /// Clave donde Edge instala el runtime; vale para equipo y para usuario.
    const CLAVE_WEBVIEW2: &str =
        r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    /// Instalador oficial (enlace estable de Microsoft).
    const DESCARGA: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

    /// Engancha `stdout`/`stderr` a la consola que lanzó el proceso.
    ///
    /// El binario de release es de subsistema Windows, así que no tiene consola
    /// propia: sin `AttachConsole` un `selftest` no imprimiría nada. Si stdout
    /// ya es válido (tubería, redirección a fichero o consola heredada) no se
    /// toca: pisarlo con `CONOUT$` rompería `"…" --selftest | more`.
    pub fn adjuntar_consola() {
        // SAFETY: `GetStdHandle` solo lee el handle del proceso; el resto de
        // llamadas reciben handles/constantes válidas y el buffer de `CONOUT$`
        // termina en NUL. Si algo falla, se sale en silencio y la CLI sigue
        // funcionando sin salida por consola.
        unsafe {
            let actual = GetStdHandle(STD_OUTPUT_HANDLE);
            if !actual.is_null() && actual != INVALID_HANDLE_VALUE {
                return;
            }
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            let nombre = ancho("CONOUT$");
            let handle: HANDLE = CreateFileW(
                nombre.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return;
            }
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }

    /// `true` si WebView2 está instalado; si no, avisa y devuelve `false`.
    #[must_use]
    pub fn webview2_listo() -> bool {
        if webview2_instalado() {
            return true;
        }

        let mensaje = format!(
            "Lunar Offline necesita Microsoft Edge WebView2 y no lo ha encontrado.\n\n\
             Descárgalo aquí:\n{DESCARGA}\n\n\
             Después vuelve a abrir Lunar Offline."
        );
        let texto = ancho(&mensaje);
        let titulo = ancho("Lunar Offline");
        // SAFETY: `texto` y `titulo` son buffers UTF-16 terminados en NUL que
        // viven hasta que el usuario cierra el diálogo.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                texto.as_ptr(),
                titulo.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
        false
    }

    /// Consulta las tres ubicaciones que documenta Microsoft.
    fn webview2_instalado() -> bool {
        let claves = [
            (
                HKEY_LOCAL_MACHINE,
                format!(r"SOFTWARE\WOW6432Node\{CLAVE_WEBVIEW2}"),
            ),
            (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{CLAVE_WEBVIEW2}")),
            (HKEY_CURRENT_USER, format!(r"SOFTWARE\{CLAVE_WEBVIEW2}")),
        ];

        claves
            .iter()
            .any(|(hkey, subclave)| pv_no_vacio(*hkey, subclave))
    }

    /// Lee el valor `pv` de una clave y dice si trae una versión.
    fn pv_no_vacio(hkey: HKEY, subclave: &str) -> bool {
        let subclave = ancho(subclave);
        let valor = ancho("pv");
        let mut buffer = [0u16; 64];
        let mut tamano = u32::try_from(buffer.len() * std::mem::size_of::<u16>()).unwrap_or(0);
        let mut tipo = 0u32;

        // SAFETY: `subclave` y `valor` terminan en NUL; `buffer` y `tamano`
        // describen la misma memoria y `tipo` apunta a un u32 válido.
        let estado = unsafe {
            RegGetValueW(
                hkey,
                subclave.as_ptr(),
                valor.as_ptr(),
                RRF_RT_REG_SZ,
                &raw mut tipo,
                buffer.as_mut_ptr().cast(),
                &raw mut tamano,
            )
        };
        if estado != 0 {
            return false;
        }

        let unidades = usize::try_from(tamano).unwrap_or(0) / 2;
        let texto = String::from_utf16_lossy(&buffer[..unidades.saturating_sub(1)]);
        !texto.trim_matches('\0').trim().is_empty()
    }

    /// Cadena UTF-16 terminada en NUL.
    fn ancho(texto: &str) -> Vec<u16> {
        OsStr::new(texto).encode_wide().chain(once(0)).collect()
    }
}
