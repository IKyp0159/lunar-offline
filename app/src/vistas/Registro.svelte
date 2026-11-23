<script lang="ts">
  import Consola from "../Consola.svelte";
  import type { Estado } from "../ipc";
  import type { Textos } from "../i18n";

  let {
    estado,
    lineas,
    avisos,
    t,
    alCopiar,
    alAbrirLogs,
    alRestaurar,
    alCopiarRuta,
  }: {
    estado: Estado;
    lineas: string[];
    avisos: string[];
    t: Textos;
    alCopiar: () => void;
    alAbrirLogs: () => void;
    alRestaurar: () => void;
    alCopiarRuta: (ruta: string) => void;
  } = $props();

  const rutas = $derived([
    { nombre: "App", ruta: estado.rutas.app },
    { nombre: "Lunar", ruta: estado.rutas.lunar },
    { nombre: "Minecraft", ruta: estado.rutas.minecraft },
    { nombre: "Logs", ruta: estado.rutas.logs },
    { nombre: "Multiver", ruta: estado.rutas.multiver },
  ]);
</script>

<section class="vista">
  <header class="cabecera-vista">
    <div>
      <h2>{t.registroTitulo}</h2>
      <p>{t.registroSubtitulo}</p>
    </div>
    <div class="acciones-cabecera">
      <button type="button" class="boton fantasma pequeno" onclick={alCopiar}>
        {t.copiar}
      </button>
      <button type="button" class="boton fantasma pequeno" onclick={alAbrirLogs}>
        {t.abrirLogs}
      </button>
      <button type="button" class="boton fantasma pequeno" onclick={alRestaurar}>
        {t.restaurar}
      </button>
    </div>
  </header>

  <div class="rejilla">
    <div class="columna">
      <section class="panel">
        <div class="panel-titulo">
          <h3>{t.avisosSalud}</h3>
          <span class="chip">{avisos.length}</span>
        </div>

        {#if avisos.length === 0}
          <p class="campo-ayuda">{t.sinAvisosSalud}</p>
        {:else}
          <ul class="lista-avisos">
            {#each avisos as aviso, indice (indice)}
              <li>{aviso}</li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="panel">
        <div class="panel-titulo">
          <h3>{t.rutas}</h3>
        </div>

        <ul class="rutas">
          {#each rutas as ruta (ruta.nombre)}
            <li>
              <span class="etiqueta">{ruta.nombre}</span>
              <span class="mono ruta" title={ruta.ruta}>{ruta.ruta}</span>
              <button
                type="button"
                class="boton fantasma pequeno"
                onclick={() => alCopiarRuta(ruta.ruta)}
              >
                {t.copiarRuta}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    </div>

    <Consola {lineas} {t} alCopiar={alCopiar} />
  </div>
</section>

<style>
  .rejilla {
    display: grid;
    grid-template-columns: minmax(300px, 0.9fr) minmax(0, 1.1fr);
    gap: var(--esp-3);
    align-items: stretch;
    flex: 1;
    min-height: 0;
  }

  .columna {
    display: flex;
    flex-direction: column;
    gap: var(--esp-3);
    min-width: 0;
    min-height: 0;
  }

  .rutas {
    display: flex;
    flex-direction: column;
    gap: var(--esp-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .rutas li {
    display: flex;
    align-items: center;
    gap: var(--esp-1);
    min-width: 0;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--borde);
  }

  .rutas li:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }

  .etiqueta {
    flex: none;
    width: 82px;
    font-size: 0.68rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--texto-tenue);
  }

  .ruta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--texto-medio);
  }

  @media (max-width: 1100px) {
    .rejilla {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
