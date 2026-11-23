/// Tipos de los avisos flotantes.
export type Tono = "neutro" | "error" | "aviso";

/// Aviso flotante; el `id` evita que Svelte reuse nodos al borrar uno del medio.
export interface Toast {
  id: number;
  texto: string;
  tono: Tono;
}
