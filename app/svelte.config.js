import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// Configuración de Svelte para la app: TypeScript en `<script lang="ts">` y en
// las expresiones de la plantilla. Vite la lee por su cuenta; `svelte-check`
// necesita este fichero para resolver el preprocesador fuera del build.
export default {
  preprocess: vitePreprocess(),
};
