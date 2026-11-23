<script lang="ts">
  import type { Textos } from "../i18n";
  import type { Vista } from "../vista";

  let {
    vista,
    instancias,
    t,
    alCambiar,
  }: {
    vista: Vista;
    instancias: number;
    t: Textos;
    alCambiar: (vista: Vista) => void;
  } = $props();

  const enlaces: { id: Vista; etiqueta: keyof Textos; tecla: string }[] = [
    { id: "jugar", etiqueta: "navJugar", tecla: "1" },
    { id: "perfiles", etiqueta: "navPerfiles", tecla: "2" },
    { id: "servidores", etiqueta: "navServidores", tecla: "3" },
    { id: "registro", etiqueta: "navRegistro", tecla: "4" },
  ];
</script>

<aside class="lateral">
  <div class="marca">
    <span class="logo" aria-hidden="true"></span>
    <div>
      <strong>LUNAR</strong>
      <span>OFFLINE</span>
    </div>
  </div>

  <nav aria-label="Vistas">
    {#each enlaces as enlace (enlace.id)}
      <button
        type="button"
        class="enlace"
        class:activo={vista === enlace.id}
        aria-current={vista === enlace.id ? "page" : undefined}
        onclick={() => alCambiar(enlace.id)}
      >
        <span class="indice" aria-hidden="true"></span>
        <span class="texto">{t[enlace.etiqueta]}</span>
        {#if enlace.id === "jugar" && instancias > 0}
          <span class="cuenta" aria-label={`${instancias} abiertas`}>{instancias}</span>
        {/if}
        <kbd aria-hidden="true">{enlace.tecla}</kbd>
      </button>
    {/each}
  </nav>

</aside>

<style>
  .lateral {
    grid-column: 1;
    grid-row: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: var(--esp-2) var(--esp-1);
    border-right: 1px solid var(--borde);
    background: var(--fondo-1);
  }

  .marca {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px var(--esp-1) var(--esp-3);
  }

  .logo {
    width: 22px;
    height: 22px;
    border: 1px solid var(--texto);
    border-radius: 50%;
    position: relative;
    flex: none;
  }

  .logo::after {
    content: "";
    position: absolute;
    inset: 6px;
    border-radius: 50%;
    background: var(--texto);
  }

  .marca div {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
  }

  .marca strong {
    font-size: 0.8rem;
    letter-spacing: 0.24em;
    font-weight: 600;
  }

  .marca span {
    font-size: 0.62rem;
    letter-spacing: 0.3em;
    color: var(--texto-tenue);
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .enlace {
    display: flex;
    align-items: center;
    gap: var(--esp-1);
    height: 36px;
    padding: 0 var(--esp-1);
    border: none;
    border-radius: var(--radio-chico);
    background: transparent;
    color: var(--texto-medio);
    text-align: left;
    transition:
      background var(--transicion),
      color var(--transicion);
  }

  .enlace:hover {
    background: var(--fondo-2);
    color: var(--texto);
  }

  .enlace.activo {
    background: var(--fondo-2);
    color: var(--texto);
  }

  .indice {
    width: 2px;
    height: 16px;
    border-radius: 2px;
    background: transparent;
    transition: background var(--transicion);
  }

  .enlace.activo .indice {
    background: var(--texto);
  }

  .texto {
    flex: 1;
    font-size: 0.82rem;
    letter-spacing: 0.04em;
  }

  .cuenta {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--texto);
    color: var(--fondo-0);
    font-size: 0.66rem;
    font-weight: 600;
  }

  kbd {
    font-family: inherit;
    font-size: 0.62rem;
    color: var(--texto-tenue);
    border: 1px solid var(--borde);
    border-radius: 3px;
    padding: 0 4px;
    line-height: 15px;
  }

</style>
