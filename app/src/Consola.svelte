<script lang="ts">
  import type { Textos } from "./i18n";

  let {
    lineas,
    t,
    alCopiar,
  }: {
    lineas: string[];
    t: Textos;
    alCopiar: () => void;
  } = $props();

  let caja: HTMLDivElement | undefined = $state();

  // Auto-scroll solo si el usuario ya estaba abajo: no le robamos la lectura.
  $effect(() => {
    const total = lineas.length;
    if (!caja || total === 0) {
      return;
    }
    const cerca = caja.scrollHeight - caja.scrollTop - caja.clientHeight < 40;
    if (cerca) {
      caja.scrollTop = caja.scrollHeight;
    }
  });
</script>

<section class="consola panel" aria-label={t.consola}>
  <header class="panel-titulo">
    <h3>{t.consola}</h3>
    <button type="button" class="boton fantasma pequeno" onclick={alCopiar}>{t.copiar}</button>
  </header>

  <div class="papel" bind:this={caja} role="log" aria-live="polite">
    {#if lineas.length === 0}
      <p class="vacio-texto">{t.consolaVacia}</p>
    {:else}
      {#each lineas as linea, indice (indice)}
        <p class="linea" class:destacada={linea.includes("Setting user")}>{linea}</p>
      {/each}
    {/if}
  </div>
</section>

<style>
  .consola {
    display: flex;
    flex-direction: column;
    gap: var(--esp-1);
    min-height: 0;
    flex: 1;
    padding: 0;
    overflow: hidden;
  }

  header {
    margin: 0;
    padding: var(--esp-2) var(--esp-2) var(--esp-1);
  }

  .papel {
    flex: 1;
    min-height: 120px;
    overflow-y: auto;
    padding: var(--esp-2);
    border-top: 1px solid var(--borde);
    background: var(--fondo-0);
    font-family: "Cascadia Mono", "Consolas", monospace;
    font-size: 0.76rem;
    line-height: 1.55;
    color: var(--texto-medio);
    user-select: text;
  }

  .linea {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .linea.destacada {
    color: var(--texto);
  }

  .vacio-texto {
    margin: 0;
    color: var(--texto-tenue);
    font-family: inherit;
  }
</style>
