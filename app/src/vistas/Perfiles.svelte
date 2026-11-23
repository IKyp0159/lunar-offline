<script lang="ts">
  import { tick } from "svelte";
  import MenuContextual from "../componentes/MenuContextual.svelte";
  import type { ItemMenu } from "../componentes/menu";
  import type { Perfil } from "../ipc";
  import type { Textos } from "../i18n";

  let {
    perfiles,
    activoId,
    t,
    alCrear,
    alActivar,
    alDuplicar,
    alBorrar,
    alRenombrar,
    alEditar,
  }: {
    perfiles: Perfil[];
    activoId: string;
    t: Textos;
    alCrear: (nombre: string) => void;
    alActivar: (id: string) => void;
    alDuplicar: (id: string) => void;
    alBorrar: (id: string) => void;
    alRenombrar: (id: string, nombre: string) => void;
    alEditar: (id: string) => void;
  } = $props();

  let busqueda = $state("");
  let creando = $state(false);
  let nombreNuevo = $state("");
  let renombrandoId = $state<string | null>(null);
  let nombreRenombrado = $state("");
  let confirmandoId = $state<string | null>(null);
  let menu = $state<{ x: number; y: number; perfil: Perfil } | null>(null);

  const itemsMenu = $derived.by<ItemMenu[]>(() => {
    if (!menu) {
      return [];
    }
    const items: ItemMenu[] = [];
    if (menu.perfil.id !== activoId) {
      items.push({ id: "activar", etiqueta: t.perfilActivar });
    }
    items.push({ id: "editar", etiqueta: t.editarEnJugar });
    items.push({ id: "duplicar", etiqueta: t.perfilDuplicar });
    items.push({ id: "renombrar", etiqueta: t.perfilRenombrar });
    items.push({ id: "borrar", etiqueta: t.borrar, peligro: true });
    return items;
  });

  let entradaNueva: HTMLInputElement | undefined = $state();
  let entradaRenombre: HTMLInputElement | undefined = $state();

  const filtrados = $derived.by(() => {
    const texto = busqueda.trim().toLowerCase();
    if (!texto) {
      return perfiles;
    }
    return perfiles.filter((perfil) =>
      [perfil.name, perfil.nick, perfil.version, perfil.server]
        .join(" ")
        .toLowerCase()
        .includes(texto),
    );
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

  async function empezarCrear(): Promise<void> {
    creando = true;
    nombreNuevo = "";
    confirmandoId = null;
    await tick();
    entradaNueva?.focus();
  }

  function crear(): void {
    const nombre = nombreNuevo.trim();
    if (!nombre) {
      return;
    }
    alCrear(nombre);
    creando = false;
    nombreNuevo = "";
  }

  async function empezarRenombrar(perfil: Perfil): Promise<void> {
    renombrandoId = perfil.id;
    nombreRenombrado = perfil.name;
    confirmandoId = null;
    await tick();
    entradaRenombre?.select();
  }

  function guardarNombre(): void {
    const nombre = nombreRenombrado.trim();
    if (!renombrandoId || !nombre) {
      renombrandoId = null;
      return;
    }
    alRenombrar(renombrandoId, nombre);
    renombrandoId = null;
  }

  function borrar(id: string): void {
    if (confirmandoId === id) {
      alBorrar(id);
      confirmandoId = null;
    } else {
      confirmandoId = id;
    }
  }

  function abrirMenu(evento: MouseEvent, perfil: Perfil): void {
    evento.preventDefault();
    menu = { x: evento.clientX, y: evento.clientY, perfil };
  }

  function elegirMenu(id: string): void {
    if (!menu) {
      return;
    }
    if (id === "activar") {
      alActivar(menu.perfil.id);
    } else if (id === "editar") {
      alEditar(menu.perfil.id);
    } else if (id === "duplicar") {
      alDuplicar(menu.perfil.id);
    } else if (id === "renombrar") {
      void empezarRenombrar(menu.perfil);
    } else if (id === "borrar") {
      borrar(menu.perfil.id);
    }
  }
</script>

<section class="vista">
  <header class="cabecera-vista">
    <div>
      <h2>{t.perfilesTitulo}</h2>
      <p>{t.perfilesSubtitulo}</p>
    </div>
    <div class="acciones-cabecera">
      <span class="chip">{perfiles.length}</span>
      <button type="button" class="boton" onclick={empezarCrear}>{t.perfilNuevo}</button>
    </div>
  </header>

  <div class="buscador">
    <input
      type="search"
      placeholder={t.buscar}
      aria-label={t.buscar}
      bind:value={busqueda}
      spellcheck="false"
    />
  </div>

  {#if creando}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="panel fila-crear"
      onsubmit={(evento) => {
        evento.preventDefault();
        crear();
      }}
      onkeydown={(evento) => {
        if (evento.key === "Escape") {
          creando = false;
        }
      }}
    >
      <label class="campo">
        <span>{t.nombrePerfil}</span>
        <input
          bind:this={entradaNueva}
          bind:value={nombreNuevo}
          maxlength="32"
          placeholder={t.perfilNuevo}
          spellcheck="false"
        />
      </label>
      <button type="submit" class="boton" disabled={!nombreNuevo.trim()}>{t.guardar}</button>
      <button type="button" class="boton fantasma" onclick={() => (creando = false)}>
        {t.cancelar}
      </button>
    </form>
  {/if}

  {#if perfiles.length === 0}
    <div class="vacio">
      <strong>{t.perfilesVacio}</strong>
      <span>{t.perfilesSubtitulo}</span>
    </div>
  {:else if filtrados.length === 0}
    <div class="vacio">
      <strong>{t.vacio}</strong>
      <span>{t.buscar}: «{busqueda}»</span>
    </div>
  {:else}
    <ul class="lista">
      {#each filtrados as perfil (perfil.id)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li
          class="fila-lista"
          class:activa={perfil.id === activoId}
          oncontextmenu={(evento) => abrirMenu(evento, perfil)}
        >
          <div class="principal">
            {#if renombrandoId === perfil.id}
              <input
                bind:this={entradaRenombre}
                bind:value={nombreRenombrado}
                maxlength="32"
                class="entrada-renombre"
                spellcheck="false"
                onkeydown={(evento) => {
                  if (evento.key === "Enter") {
                    evento.preventDefault();
                    guardarNombre();
                  } else if (evento.key === "Escape") {
                    renombrandoId = null;
                  }
                }}
              />
            {:else}
              <strong>{perfil.name}</strong>
              <span>
                {perfil.nick.trim() || t.sinNick} · {perfil.version} · {perfil.ram} MB
              </span>
              <span>
                {perfil.server.trim() || t.sinServidor} · {t.perfilUltimoUso}:
                {fecha(perfil.last_used_at)}
              </span>
            {/if}
          </div>

          <div class="acciones-fila">
            {#if renombrandoId === perfil.id}
              <button type="button" class="boton pequeno" onclick={guardarNombre}>
                {t.guardar}
              </button>
              <button
                type="button"
                class="boton fantasma pequeno"
                onclick={() => (renombrandoId = null)}
              >
                {t.cancelar}
              </button>
            {:else}
              {#if perfil.id === activoId}
                <span class="chip activo">{t.perfilActivo}</span>
              {:else}
                <button
                  type="button"
                  class="boton fantasma pequeno"
                  onclick={() => alActivar(perfil.id)}
                >
                  {t.perfilActivar}
                </button>
              {/if}
              <button
                type="button"
                class="boton fantasma pequeno"
                onclick={() => alEditar(perfil.id)}
              >
                {t.editarEnJugar}
              </button>
              <button
                type="button"
                class="boton fantasma pequeno"
                onclick={() => alDuplicar(perfil.id)}
              >
                {t.perfilDuplicar}
              </button>
              <button
                type="button"
                class="boton fantasma pequeno"
                onclick={() => empezarRenombrar(perfil)}
              >
                {t.perfilRenombrar}
              </button>
              <button
                type="button"
                class="boton peligro pequeno"
                aria-label={t.perfilBorrar}
                onclick={() => borrar(perfil.id)}
              >
                {confirmandoId === perfil.id ? t.confirmarBorrar : t.borrar}
              </button>
            {/if}
          </div>

          {#if confirmandoId === perfil.id && perfil.id === activoId}
            <p class="aviso-ambar" role="alert">{t.perfilBorrarPrincipal}</p>
          {/if}
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
  .fila-crear {
    display: flex;
    align-items: flex-end;
    gap: var(--esp-2);
    flex: none;
  }

  .fila-crear .campo {
    flex: 1;
  }

  .entrada-renombre {
    width: 100%;
    min-height: 30px;
    padding: 4px 8px;
    border: 1px solid var(--texto);
    border-radius: var(--radio-chico);
    background: var(--fondo-2);
    color: var(--texto);
  }

  .acciones-fila {
    flex-wrap: wrap;
    justify-content: flex-end;
    max-width: 55%;
  }

  .fila-lista {
    align-items: flex-start;
  }

  @media (max-width: 1050px) {
    .acciones-fila {
      max-width: 45%;
    }
  }
</style>
