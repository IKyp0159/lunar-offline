<script lang="ts">
  import type { EstadoJuego } from "../ipc";
  import type { Textos } from "../i18n";

  let {
    estado,
    launcherOficial,
    instancias,
    version,
    t,
  }: {
    estado: EstadoJuego;
    launcherOficial: boolean;
    instancias: number;
    version: string;
    t: Textos;
  } = $props();

  const etiqueta = $derived.by(() => {
    switch (estado.fase) {
      case "jugando":
        return `${t.jugando} · ${estado.nick}`;
      case "jugando_varios":
        return `${t.jugando} · ${estado.cantidad} instancias`;
      case "preparando":
        return t.preparando;
      case "restaurando":
        return t.restaurando;
      case "error":
        return estado.mensaje;
      default:
        return t.listoParaJugar;
    }
  });

  const activo = $derived(
    estado.fase === "jugando" ||
      estado.fase === "jugando_varios" ||
      estado.fase === "preparando",
  );
</script>

<footer class="barra" aria-live="polite">
  <span class="punto" class:activo class:error={estado.fase === "error"}></span>
  <span class="estado" title={etiqueta}>{etiqueta}</span>

  <span class="relleno"></span>

  {#if instancias > 0}
    <span class="dato">{instancias} × {t.instancias.toLowerCase()}</span>
  {/if}

  <span class="dato" class:aviso={launcherOficial}>
    {launcherOficial ? t.launcherOficial : t.launcherCerrado}
  </span>

  <span class="version">v{version}</span>
</footer>

<style>
  .barra {
    grid-column: 1 / -1;
    grid-row: 2;
    display: flex;
    align-items: center;
    gap: var(--esp-2);
    height: var(--barra-estado);
    padding: 0 var(--esp-2);
    border-top: 1px solid var(--borde);
    background: var(--fondo-1);
    font-size: 0.72rem;
    color: var(--texto-tenue);
    white-space: nowrap;
    overflow: hidden;
  }

  .punto {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--texto-tenue);
    flex: none;
    transition: background var(--transicion);
  }

  .punto.activo {
    background: var(--texto);
    animation: latido 1.8s ease-in-out infinite;
  }

  .punto.error {
    background: var(--error);
    animation: none;
  }

  @keyframes latido {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
  }

  .estado {
    max-width: 42ch;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--texto-medio);
  }

  .relleno {
    flex: 1;
  }

  .dato {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dato.aviso {
    color: var(--aviso);
  }

  .version {
    color: var(--texto-tenue);
    letter-spacing: 0.08em;
  }
</style>
