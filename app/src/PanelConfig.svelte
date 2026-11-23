<script lang="ts">
  import type { Config, Version } from "./ipc";
  import type { Textos } from "./i18n";

  let {
    config = $bindable(),
    versiones,
    javaDetectado,
    t,
  }: {
    config: Config;
    versiones: Version[];
    javaDetectado: string | null;
    t: Textos;
  } = $props();

  const RAM_MIN = 1024;
  const RAM_MAX = 8192;
  const RAM_PASO = 256;

  const nickValido = $derived(/^[A-Za-z0-9_]{1,16}$/.test(config.nick.trim()));

  // Un arranque limpio no debe salir en rojo: el aviso del nick aparece cuando
  // hay algo escrito o cuando el usuario ya lo ha tocado (al salir del campo).
  let nickTocado = $state(false);
  const nickInvalido = $derived(
    !nickValido && (nickTocado || config.nick.trim().length > 0),
  );
  const ramValida = $derived(
    config.ram >= RAM_MIN && config.ram <= RAM_MAX && config.ram % RAM_PASO === 0,
  );
  const resolucionValida = $derived(
    config.width >= 320 && config.width <= 7680 && config.height >= 240 && config.height <= 4320,
  );

  const opcionesVersion = $derived.by(() => {
    const nombres = versiones.map((version) => version.nombre);
    if (config.version && !nombres.includes(config.version)) {
      return [config.version, ...nombres];
    }
    return nombres;
  });

  const mensaje = $derived(
    nickInvalido
      ? t.errorNick
      : !ramValida
        ? t.errorRam
        : !resolucionValida
          ? t.errorResolucion
          : null,
  );

  // El slider mueve la RAM en pasos de 256 MB.
  function ajustarRam(valor: number): void {
    const paso = Math.round(valor / RAM_PASO) * RAM_PASO;
    config.ram = Math.min(RAM_MAX, Math.max(RAM_MIN, paso));
  }
</script>

<form class="formulario" onsubmit={(evento) => evento.preventDefault()}>
  <div class="fila-campos">
    <label class="campo">
      <span>{t.cuenta}</span>
      <input
        type="text"
        bind:value={config.nick}
        maxlength="16"
        spellcheck="false"
        autocomplete="off"
        class:invalido={nickInvalido}
        aria-invalid={nickInvalido}
        onblur={() => (nickTocado = true)}
        placeholder="Steve"
      />
    </label>

    <label class="campo">
      <span>{t.version}</span>
      <select bind:value={config.version}>
        {#each opcionesVersion as nombre (nombre)}
          <option value={nombre}>{nombre}</option>
        {/each}
      </select>
    </label>
  </div>

  <label class="campo">
    <span>{t.servidor}</span>
    <input
      type="text"
      bind:value={config.server}
      spellcheck="false"
      autocomplete="off"
      placeholder="play.ejemplo.net"
    />
  </label>

  <div class="campo">
    <span>{t.ram} · {config.ram} MB</span>
    <input
      type="range"
      min={RAM_MIN}
      max={RAM_MAX}
      step={RAM_PASO}
      value={config.ram}
      oninput={(evento) => ajustarRam(Number(evento.currentTarget.value))}
      class:invalido={!ramValida}
      aria-invalid={!ramValida}
    />
  </div>

  <div class="campo">
    <span>{t.resolucion}</span>
    <div class="resolucion">
      <input
        type="number"
        bind:value={config.width}
        min="320"
        max="7680"
        class:invalido={!resolucionValida}
        aria-invalid={!resolucionValida}
      />
      <span class="por">×</span>
      <input
        type="number"
        bind:value={config.height}
        min="240"
        max="4320"
        class:invalido={!resolucionValida}
        aria-invalid={!resolucionValida}
      />
    </div>
  </div>

  <label class="interruptor">
    <input type="checkbox" bind:checked={config.fullscreen} />
    <span>{t.pantallaCompleta}</span>
  </label>

  <button
    type="button"
    class="desplegable"
    aria-expanded={config.advanced}
    onclick={() => (config.advanced = !config.advanced)}
  >
    <span>{t.avanzado}</span>
    <span class="flecha" class:abierta={config.advanced} aria-hidden="true">▾</span>
  </button>

  {#if config.advanced}
    <label class="campo">
      <span>{t.java}</span>
      <input
        type="text"
        bind:value={config.java}
        spellcheck="false"
        autocomplete="off"
        placeholder={javaDetectado ?? t.javaAuto}
      />
      <span class="campo-ayuda">{javaDetectado ?? t.javaAuto}</span>
    </label>
  {/if}

  {#if mensaje}
    <p class="aviso-error" role="alert">{mensaje}</p>
  {/if}
</form>

<style>
  .formulario {
    display: flex;
    flex-direction: column;
    gap: var(--esp-2);
  }

  .resolucion {
    display: flex;
    align-items: center;
    gap: var(--esp-1);
  }

  .resolucion input {
    width: 100%;
    min-height: 32px;
    padding: 6px 10px;
    border: 1px solid var(--borde);
    border-radius: var(--radio-chico);
    background: var(--fondo-1);
    color: var(--texto);
    transition: border-color var(--transicion);
  }

  .resolucion input:hover {
    border-color: var(--borde-fuerte);
  }

  .resolucion input:focus {
    border-color: var(--texto);
    outline: none;
  }

  .resolucion input.invalido {
    border-color: var(--error);
  }

  .por {
    color: var(--texto-tenue);
  }

  input[type="range"] {
    width: 100%;
    accent-color: var(--texto);
  }

  .desplegable {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--esp-2) 0 0;
    border: none;
    border-top: 1px solid var(--borde);
    background: none;
    color: var(--texto-medio);
    font-size: 0.72rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    transition: color var(--transicion);
  }

  .desplegable:hover {
    color: var(--texto);
  }

  .flecha {
    transition: transform var(--transicion);
  }

  .flecha.abierta {
    transform: rotate(180deg);
  }
</style>
