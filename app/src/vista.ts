/// Vistas de la aplicación. El orden es el de la barra lateral y el de
/// `Ctrl+1..4`.
export type Vista = "jugar" | "perfiles" | "servidores" | "registro";

/// Orden estable para atajos y navegación.
export const VISTAS: Vista[] = ["jugar", "perfiles", "servidores", "registro"];
