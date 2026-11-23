<script lang="ts">
  import { onMount } from "svelte";
  import BarraLateral from "./componentes/BarraLateral.svelte";
  import BarraEstado from "./componentes/BarraEstado.svelte";
  import Toast from "./componentes/Toast.svelte";
  import type { Toast as ToastItem, Tono } from "./componentes/toast";
  import {
    activarPerfil,
    alCambiarEstado,
    alRecibirAviso,
    alRecibirLog,
    borrarPerfil,
    borrarServidor,
    crearPerfil,
    duplicarPerfil,
    guardarConfig,
    guardarServidor,
    lanzarJuego,
    pedirDiagnostico,
    pedirEstado,
    renombrarPerfil,
    restaurarCuenta,
    rutaCarpeta,
    type Aviso,
    type Config,
    type Estado,
    type EstadoJuego,
    type Servidor,
  } from "./ipc";
  import { textos } from "./i18n";
  import { VISTAS, type Vista } from "./vista";
  import Jugar from "./vistas/Jugar.svelte";
  import Perfiles from "./vistas/Perfiles.svelte";
  import Registro from "./vistas/Registro.svelte";
  import Servidores from "./vistas/Servidores.svelte";

  const t = textos("es");
  const MAX_LINEAS = 800;

  let estado = $state<Estado | null>(null);
  let config = $state<Config | null>(null);
  let fase = $state<EstadoJuego>({ fase: "inactivo" });
  let vista = $state<Vista>("jugar");
  let lineas = $state<string[]>([]);
  let salud = $state<string[]>([]);
  let toasts = $state<ToastItem[]>([]);
  let ocupado = $state(false);
  let guardado = $state(false);
  let error = $state<string | null>(null);
  let siguienteToast = 0;

  const instancias = $derived(estado?.instancias ?? []);
  const entornoListo = $derived(estado?.entorno.listo ?? false);
  // La versión se anuncia como v2.0 aunque el paquete lleve el parche (2.0.0).
  const versionApp = $derived.by(() => {
    const partes = (estado?.version_app ?? "2.0.0").split(".");
    return `${partes[0] ?? "2"}.${partes[1] ?? "0"}`;
  });

  const puedeJugar = $derived.by(() => {
    if (!config || ocupado) {
      return false;
    }
    const nickValido = /^[A-Za-z0-9_]{1,16}$/.test(config.nick.trim());
    const ramValida =
      config.ram >= 1024 && config.ram <= 8192 && config.ram % 256 === 0;
    const resolucionValida =
      config.width >= 320 &&
      config.width <= 7680 &&
      config.height >= 240 &&
      config.height <= 4320;
    return (
      nickValido &&
      ramValida &&
      resolucionValida &&
      fase.fase !== "preparando" &&
      fase.fase !== "restaurando" &&
      entornoListo &&
      !(estado?.launcher_oficial ?? false)
    );
  });

  function notificar(texto: string, tono: Tono = "neutro"): void {
    const id = ++siguienteToast;
    toasts = [...toasts.slice(-3), { id, texto, tono }];
    setTimeout(() => cerrarToast(id), 4200);
  }

  function cerrarToast(id: number): void {
    toasts = toasts.filter((toast) => toast.id !== id);
  }

  function anotar(texto: string): void {
    lineas = [...lineas, texto].slice(-MAX_LINEAS);
  }

  function anotarAviso(aviso: Aviso): void {
    salud = [...salud, `${aviso.mensaje} — ${aviso.pista}`].slice(-8);
    anotar(`[aviso] ${aviso.mensaje}`);
  }

  function aplicar(datos: Estado): void {
    estado = datos;
    config = datos.config;
    fase = datos.estado;
  }

  async function cargar(): Promise<void> {
    try {
      const datos = await pedirEstado();
      aplicar(datos);
      if (datos.recuperacion) {
        anotar(`[cuenta] ${datos.recuperacion}`);
      }
      if (datos.aviso) {
        anotar(`[config] ${datos.aviso}`);
      }
    } catch (fallo) {
      error = String(fallo);
      notificar(String(fallo), "error");
    }
  }

  async function refrescarEstado(): Promise<void> {
    try {
      const datos = await pedirEstado();
      if (!estado) {
        aplicar(datos);
        return;
      }
      estado.instancias = datos.instancias;
      estado.entorno = datos.entorno;
      estado.launcher_oficial = datos.launcher_oficial;
      fase = datos.estado;
    } catch {
      // Un refresco fallido no debe tapar la partida en curso.
    }
  }

  async function jugar(): Promise<void> {
    if (!config || !puedeJugar) {
      return;
    }
    ocupado = true;
    error = null;
    salud = [];
    try {
      const juego = await lanzarJuego({
        nick: config.nick.trim(),
        version: config.version,
        servidor: config.server.trim(),
        ram_mb: config.ram,
        ancho: Number(config.width),
        alto: Number(config.height),
        pantalla_completa: config.fullscreen,
        java: config.java.trim() === "" ? null : config.java.trim(),
      });
      anotar(`[juego] pid ${juego.pid} · ${juego.version} · ${juego.nick}`);
      notificar(`${t.jugando}: ${juego.nick}`);
      await refrescarEstado();
    } catch (fallo) {
      error = String(fallo);
      fase = { fase: "error", mensaje: String(fallo) };
      notificar(String(fallo), "error");
    } finally {
      ocupado = false;
    }
  }

  async function guardar(): Promise<void> {
    if (!config) {
      return;
    }
    try {
      aplicar(await guardarConfig({ ...config }));
      guardado = true;
      setTimeout(() => (guardado = false), 1500);
      anotar("[config] guardada");
    } catch (fallo) {
      error = String(fallo);
      notificar(String(fallo), "error");
    }
  }

  async function restaurar(): Promise<void> {
    try {
      const resultado = await restaurarCuenta();
      const texto = resultado ?? t.sinRestaurar;
      anotar(`[cuenta] ${texto}`);
      notificar(texto);
    } catch (fallo) {
      error = String(fallo);
      notificar(String(fallo), "error");
    }
  }

  async function copiarInforme(): Promise<void> {
    try {
      const informe = await pedirDiagnostico();
      await navigator.clipboard.writeText(informe);
      anotar(`[soporte] ${t.copiado}`);
      notificar(t.copiado);
    } catch (fallo) {
      error = String(fallo);
      notificar(String(fallo), "error");
    }
  }

  async function abrirLogs(): Promise<void> {
    try {
      const ruta = await rutaCarpeta("logs");
      notificar(`${t.abrirLogs}: ${ruta}`);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function copiarRuta(ruta: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(ruta);
      notificar(t.rutaCopiada);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function crear(nombre: string): Promise<void> {
    try {
      aplicar(await crearPerfil(nombre));
      notificar(`${t.perfilNuevo}: ${nombre}`);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function activar(id: string): Promise<void> {
    try {
      aplicar(await activarPerfil(id));
      notificar(t.perfilActivo);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function duplicar(id: string): Promise<void> {
    try {
      aplicar(await duplicarPerfil(id));
      notificar(t.perfilDuplicar);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function eliminarPerfil(id: string): Promise<void> {
    try {
      aplicar(await borrarPerfil(id));
      notificar(t.borrar);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function renombrar(id: string, nombre: string): Promise<void> {
    try {
      aplicar(await renombrarPerfil(id, nombre));
      notificar(t.perfilRenombrar);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function editarPerfil(id: string): Promise<void> {
    try {
      if (config?.active_profile !== id) {
        aplicar(await activarPerfil(id));
      }
      vista = "jugar";
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function guardarServidorAccion(servidor: Servidor): Promise<void> {
    try {
      aplicar(await guardarServidor(servidor));
      notificar(t.servidorGuardar);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  async function eliminarServidor(id: string): Promise<void> {
    try {
      aplicar(await borrarServidor(id));
      notificar(t.borrar);
    } catch (fallo) {
      notificar(String(fallo), "error");
    }
  }

  function conexionServidor(servidor: Servidor): string {
    const host = servidor.address.trim();
    const yaTienePuerto =
      /^\[[^\]]+\]:\d+$/.test(host) || /^[^:\[\]]+:\d+$/.test(host);
    if (yaTienePuerto) {
      return host;
    }
    const base = host.includes(":") ? `[${host}]` : host;
    if (servidor.port === 25565 && !host.includes(":")) {
      return base;
    }
    return `${base}:${servidor.port}`;
  }

  function usarServidor(servidor: Servidor): void {
    if (!config) {
      return;
    }
    config.server = conexionServidor(servidor);
    vista = "jugar";
    notificar(`${t.servidor}: ${config.server}`);
  }

  function esElementoDeTexto(objetivo: EventTarget | null): boolean {
    return (
      objetivo instanceof HTMLInputElement ||
      objetivo instanceof HTMLTextAreaElement ||
      objetivo instanceof HTMLSelectElement ||
      (objetivo instanceof HTMLElement && objetivo.isContentEditable)
    );
  }

  function alPulsar(evento: KeyboardEvent): void {
    const escribiendo = esElementoDeTexto(evento.target);

    if (evento.key === "Escape") {
      if (toasts.length > 0) {
        toasts = [];
      } else if (escribiendo && evento.target instanceof HTMLElement) {
        evento.target.blur();
      }
      return;
    }

    if (escribiendo) {
      return;
    }

    if (evento.ctrlKey || evento.metaKey) {
      const indice = Number.parseInt(evento.key, 10);
      const destino = VISTAS[indice - 1];
      if (destino) {
        evento.preventDefault();
        vista = destino;
        return;
      }
      if (evento.key === "Enter") {
        evento.preventDefault();
        void jugar();
        return;
      }
    }

    if (evento.key === "F5") {
      evento.preventDefault();
      void cargar();
    }
  }

  onMount(() => {
    void cargar();

    const alCambiarVisibilidad = (): void => {
      document.documentElement.classList.toggle(
        "oculto",
        document.visibilityState === "hidden",
      );
    };
    document.addEventListener("visibilitychange", alCambiarVisibilidad);
    alCambiarVisibilidad();

    const suscripciones = Promise.all([
      alCambiarEstado((nuevo) => {
        fase = nuevo;
        if (nuevo.fase === "error") {
          error = nuevo.mensaje;
          notificar(nuevo.mensaje, "error");
        }
        void refrescarEstado();
      }),
      alRecibirLog(anotar),
      alRecibirAviso(anotarAviso),
    ]);

    return () => {
      document.removeEventListener("visibilitychange", alCambiarVisibilidad);
      void suscripciones.then((listas) => listas.forEach((soltar) => soltar()));
    };
  });
</script>

<svelte:window onkeydown={alPulsar} />

<div class="aplicacion">
  <BarraLateral
    {vista}
    instancias={instancias.length}
    {t}
    alCambiar={(nueva) => (vista = nueva)}
  />

  <main class="contenido">
    {#if !estado || !config}
      <div class="vista">
        <p class="campo-ayuda">{error ?? t.cargando}</p>
      </div>
    {:else if vista === "jugar"}
      <Jugar
        bind:config
        {estado}
        {fase}
        {ocupado}
        {guardado}
        {error}
        {puedeJugar}
        {t}
        alJugar={jugar}
        alGuardar={guardar}
        alRestaurar={restaurar}
      />
    {:else if vista === "perfiles"}
      <Perfiles
        perfiles={config.profiles}
        activoId={config.active_profile}
        {t}
        alCrear={crear}
        alActivar={activar}
        alDuplicar={duplicar}
        alBorrar={eliminarPerfil}
        alRenombrar={renombrar}
        alEditar={editarPerfil}
      />
    {:else if vista === "servidores"}
      <Servidores
        servidores={config.servers}
        versiones={estado.versiones}
        {t}
        alGuardar={guardarServidorAccion}
        alBorrar={eliminarServidor}
        alUsar={usarServidor}
      />
    {:else}
      <Registro
        {estado}
        {lineas}
        avisos={salud}
        {t}
        alCopiar={copiarInforme}
        alAbrirLogs={abrirLogs}
        alRestaurar={restaurar}
        alCopiarRuta={copiarRuta}
      />
    {/if}
  </main>

  <BarraEstado
    estado={fase}
    launcherOficial={estado?.launcher_oficial ?? false}
    instancias={instancias.length}
    version={versionApp}
    {t}
  />
</div>

<Toast {toasts} alCerrar={cerrarToast} />
