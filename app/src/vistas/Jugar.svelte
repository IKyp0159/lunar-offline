<script lang="ts">
  import PanelConfig from "../PanelConfig.svelte";
  import type { Config, Estado, EstadoJuego } from "../ipc";
  import type { Textos } from "../i18n";

  let {
    config = $bindable(),
    estado,
    fase,
    ocupado,
    guardado,
    error,
    puedeJugar,
    t,
    alJugar,
    alGuardar,
    alRestaurar,
  }: {
    config: Config;
    estado: Estado;
    fase: EstadoJuego;
    ocupado: boolean;
    guardado: boolean;
    error: string | null;
    puedeJugar: boolean;
    t: Textos;
    alJugar: () => void;
    alGuardar: () => void;
    alRestaurar: () => void;
  } = $props();

  const perfilActivo = $derived(
    config.profiles.find((perfil) => perfil.id === config.active_profile) ??
      config.profiles[0],
  );

  const etiquetaBoton = $derived(
    estado.instancias.length > 0 ? t.lanzarOtra : t.jugar,
  );

</script>

<section class="vista">
  <header class="cabecera-vista">
    <div>
      <h2>{t.jugarTitulo}</h2>
      <p>{t.jugarSubtitulo}</p>
    </div>
    {#if perfilActivo}
      <span class="chip activo">{perfilActivo.name}</span>
    {/if}
  </header>

  {#if error}
    <p class="aviso-error" role="alert">{error}</p>
  {/if}

  {#if estado.launcher_oficial}
    <p class="aviso-ambar" role="status">{t.launcherOficial}</p>
  {/if}

  {#if !estado.entorno.listo}
    <p class="aviso-error" role="status">{t.entornoIncompleto}</p>
  {/if}

  <div class="rejilla">
    <section class="panel">
      <div class="panel-titulo">
        <h3>{t.cuentaActiva}{perfilActivo ? ` · ${perfilActivo.name}` : ""}</h3>
        {#if fase.fase === "jugando"}
          <span class="chip activo">{fase.nick}</span>
        {:else if fase.fase === "jugando_varios"}
          <span class="chip activo">{fase.cantidad}</span>
        {/if}
      </div>

      <PanelConfig
        bind:config
        versiones={estado.versiones}
        javaDetectado={estado.entorno.java}
        {t}
      />
    </section>

    <div class="columna">
      <section class="panel">
        <div class="panel-titulo">
          <h3>{t.instancias}</h3>
          <span class="chip">{estado.instancias.length}</span>
        </div>

        {#if estado.instancias.length === 0}
          <div class="vacio">
            <strong>{t.instanciasVacias}</strong>
            <span>{t.notaNickFijo}</span>
          </div>
        {:else}
          <ul class="lista">
            {#each estado.instancias as juego (juego.pid)}
              <li class="fila-lista">
                <div class="principal">
                  <strong>{juego.nick}</strong>
                  <span>{juego.version} · {t.instanciaPid} {juego.pid}</span>
                </div>
                <span class="chip activo">{t.jugando}</span>
              </li>
            {/each}
          </ul>
          <p class="campo-ayuda margen-arriba">{t.notaNickFijo}</p>
        {/if}
      </section>
    </div>
  </div>

  <div class="acciones">
    <button type="button" class="boton" disabled={!puedeJugar} onclick={alJugar}>
      {ocupado ? t.preparando : etiquetaBoton}
    </button>
    <button type="button" class="boton fantasma" onclick={alGuardar}>
      {guardado ? t.guardado : t.guardar}
    </button>
    <button type="button" class="boton fantasma" onclick={alRestaurar}>
      {t.restaurar}
    </button>
  </div>
</section>

<style>
  .rejilla {
    display: grid;
    grid-template-columns: minmax(0, 1.05fr) minmax(280px, 0.95fr);
    gap: var(--esp-3);
    align-items: start;
    flex: 1;
    min-height: 0;
  }

  .columna {
    display: flex;
    flex-direction: column;
    gap: var(--esp-3);
    min-width: 0;
  }

  .acciones {
    display: flex;
    gap: var(--esp-1);
    flex: none;
  }

  .acciones .boton:first-child {
    min-width: 200px;
  }

  @media (max-width: 1100px) {
    .rejilla {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
