<script lang="ts">
  import { tick } from "svelte";
  import MenuContextual from "../componentes/MenuContextual.svelte";
  import type { ItemMenu } from "../componentes/menu";
  import type { Servidor, Version } from "../ipc";
  import type { Textos } from "../i18n";

  let {
    servidores,
    versiones,
    t,
    alGuardar,
    alBorrar,
    alUsar,
  }: {
    servidores: Servidor[];
    versiones: Version[];
    t: Textos;
    alGuardar: (servidor: Servidor) => void;
    alBorrar: (id: string) => void;
    alUsar: (servidor: Servidor) => void;
  } = $props();

  let editando = $state(false);
  let errorFormulario = $state<string | null>(null);
  let confirmandoId = $state<string | null>(null);
  let formulario = $state<Servidor>(vacio());
  let entradaNombre: HTMLInputElement | undefined = $state();
  let menu = $state<{ x: number; y: number; servidor: Servidor } | null>(null);

  const itemsMenu = $derived.by<ItemMenu[]>(() => [
    { id: "usar", etiqueta: t.usarServidor },
    { id: "editar", etiqueta: t.editar },
    { id: "borrar", etiqueta: t.borrar, peligro: true },
  ]);

  function vacio(): Servidor {
    return {
      id: "",
      name: "",
      address: "",
      port: 25565,
      version: "",
      favourite: false,
      last_connected: null,
      source: "manual",
      dat_key: null,
    };
  }

  const ordenados = $derived.by(() =>
    [...servidores].sort((a, b) => {
      if (a.favourite !== b.favourite) {
        return a.favourite ? -1 : 1;
      }
      return a.name.localeCompare(b.name, "es");
    }),
  );

  const opcionesVersion = $derived.by(() => {
    const nombres = versiones.map((version) => version.nombre);
    if (formulario.version && !nombres.includes(formulario.version)) {
      return [formulario.version, ...nombres];
    }
    return nombres;
  });

  const formatoFecha = new Intl.DateTimeFormat("es", {
    dateStyle: "short",
    timeStyle: "short",
  });

  function fecha(iso: string | null): string {
    if (!iso) {
      return t.nunca;
    }
    const valor = new Date(iso);
    return Number.isNaN(valor.getTime()) ? iso : formatoFecha.format(valor);
  }

  function direccion(servidor: Servidor): string {
    const host = servidor.address.trim();
    const yaTienePuerto =
      /^\[[^\]]+\]:\d+$/.test(host) || /^[^:\[\]]+:\d+$/.test(host);
    if (yaTienePuerto) {
      return host;
    }
    const base = host.includes(":") ? `[${host}]` : host;
    return `${base}:${servidor.port}`;
  }

  async function abrirNuevo(): Promise<void> {
    formulario = vacio();
    editando = true;
    errorFormulario = null;
    confirmandoId = null;
    await tick();
    entradaNombre?.focus();
  }

  async function abrirEditar(servidor: Servidor): Promise<void> {
    formulario = { ...servidor };
    editando = true;
    errorFormulario = null;
    confirmandoId = null;
    await tick();
    entradaNombre?.select();
  }

  function cerrar(): void {
    editando = false;
    errorFormulario = null;
  }

  function guardar(): void {
    const address = formulario.address.trim();
    if (!address) {
      errorFormulario = t.direccionVacia;
      return;
    }
    if (!Number.isInteger(formulario.port) || formulario.port < 1 || formulario.port > 65535) {
      errorFormulario = t.puertoInvalido;
      return;
    }
    alGuardar({
      ...formulario,
      name: formulario.name.trim(),
      address,
    });
    editando = false;
    errorFormulario = null;
  }

  function borrar(id: string): void {
    if (confirmandoId === id) {
      alBorrar(id);
      confirmandoId = null;
    } else {
      confirmandoId = id;
    }
  }

  function alternarFavorito(servidor: Servidor): void {
    alGuardar({ ...servidor, favourite: !servidor.favourite });
  }

  function abrirMenu(evento: MouseEvent, servidor: Servidor): void {
    evento.preventDefault();
    menu = { x: evento.clientX, y: evento.clientY, servidor };
  }

  function elegirMenu(id: string): void {
    if (!menu) {
      return;
    }
    if (id === "usar") {
      alUsar(menu.servidor);
    } else if (id === "editar") {
      void abrirEditar(menu.servidor);
    } else if (id === "borrar") {
      borrar(menu.servidor.id);
    }
  }
</script>

<section class="vista">
  <header class="cabecera-vista">
    <div>
      <h2>{t.servidoresTitulo}</h2>
      <p>{t.servidoresSubtitulo}</p>
    </div>
    <div class="acciones-cabecera">
      <span class="chip">{servidores.length}</span>
      <button type="button" class="boton" onclick={abrirNuevo}>{t.servidorNuevo}</button>
    </div>
  </header>

  {#if editando}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="panel formulario"
      onsubmit={(evento) => {
        evento.preventDefault();
        guardar();
      }}
      onkeydown={(evento) => {
        if (evento.key === "Escape") {
          cerrar();
        }
      }}
    >
      <div class="panel-titulo">
        <h3>{formulario.id ? t.servidorEditar : t.servidorNuevo}</h3>
        <span class="chip">{formulario.id ? "ID" : t.servidorFavorito}</span>
      </div>

      <div class="fila-campos">
        <label class="campo">
          <span>{t.servidorNombre}</span>
          <input
            bind:this={entradaNombre}
            bind:value={formulario.name}
            maxlength="48"
            spellcheck="false"
            placeholder={t.servidorNombre}
          />
        </label>

        <label class="campo">
          <span>{t.servidorDireccion}</span>
          <input
            bind:value={formulario.address}
            spellcheck="false"
            autocomplete="off"
            placeholder="play.ejemplo.net"
          />
        </label>

        <label class="campo">
          <span>{t.servidorPuerto}</span>
          <input type="number" bind:value={formulario.port} min="1" max="65535" />
        </label>

        <label class="campo">
          <span>{t.version}</span>
          <select bind:value={formulario.version}>
            <option value="">{t.sinVersion}</option>
            {#each opcionesVersion as nombre (nombre)}
              <option value={nombre}>{nombre}</option>
            {/each}
          </select>
        </label>
      </div>

      <label class="interruptor">
        <input type="checkbox" bind:checked={formulario.favourite} />
        <span>{t.servidorFavorito}</span>
      </label>

      {#if errorFormulario}
        <p class="aviso-error" role="alert">{errorFormulario}</p>
      {/if}

      <div class="acciones-formulario">
        <button type="submit" class="boton">{t.servidorGuardar}</button>
        <button type="button" class="boton fantasma" onclick={cerrar}>
          {t.servidorCancelar}
        </button>
      </div>
    </form>
  {/if}

  {#if servidores.length === 0}
    <div class="vacio">
      <strong>{t.servidoresVacio}</strong>
      <span>{t.servidoresSubtitulo}</span>
    </div>
  {:else}
    <ul class="lista">
      {#each ordenados as servidor (servidor.id)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li
          class="fila-lista"
          class:activa={confirmandoId === servidor.id}
          oncontextmenu={(evento) => abrirMenu(evento, servidor)}
        >
          <button
            type="button"
            class="estrella"
            class:marcada={servidor.favourite}
            aria-pressed={servidor.favourite}
            aria-label={t.servidorFavorito}
            onclick={() => alternarFavorito(servidor)}
          >
            {servidor.favourite ? "★" : "☆"}
          </button>

          <div class="principal">
            <strong>{servidor.name || servidor.address}</strong>
            <span>{direccion(servidor)}{servidor.version ? ` · ${servidor.version}` : ""}</span>
            <span>{t.perfilUltimoUso}: {fecha(servidor.last_connected)}</span>
          </div>

          {#if servidor.source === "servers_dat"}
            <span class="chip">{t.importado}</span>
          {/if}

          <div class="acciones-fila">
            <button type="button" class="boton pequeno" onclick={() => alUsar(servidor)}>
              {t.usarServidor}
            </button>
            <button type="button" class="boton fantasma pequeno" onclick={() => abrirEditar(servidor)}>
              {t.editar}
            </button>
            <button
              type="button"
              class="boton peligro pequeno"
              onclick={() => borrar(servidor.id)}
            >
              {confirmandoId === servidor.id ? t.confirmarBorrar : t.borrar}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  {#if menu}
    <MenuContextual
      x={menu.x}
      y={menu.y}
      items={itemsMenu}
      alElegir={elegirMenu}
      alCerrar={() => (menu = null)}
    />
  {/if}
</section>

<style>
  .formulario {
    display: flex;
    flex-direction: column;
    gap: var(--esp-2);
    flex: none;
  }

  .acciones-formulario {
    display: flex;
    gap: var(--esp-1);
  }

  .fila-lista {
    align-items: flex-start;
  }

  .acciones-fila {
    flex-wrap: wrap;
    justify-content: flex-end;
  }
</style>
