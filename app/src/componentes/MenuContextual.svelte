<script lang="ts">
  import { onMount } from "svelte";
  import type { ItemMenu } from "./menu";

  let {
    x,
    y,
    items,
    alElegir,
    alCerrar,
  }: {
    x: number;
    y: number;
    items: ItemMenu[];
    alElegir: (id: string) => void;
    alCerrar: () => void;
  } = $props();

  let menu: HTMLDivElement | undefined = $state();

  const ancho = 208;
  const alto = $derived(items.length * 34 + 12);
  const izquierda = $derived(Math.max(8, Math.min(x, window.innerWidth - ancho - 8)));
  const arriba = $derived(Math.max(8, Math.min(y, window.innerHeight - alto - 8)));

  onMount(() => {
    menu?.querySelector("button")?.focus();

    const alPulsarRaton = (evento: MouseEvent): void => {
      if (menu && evento.target instanceof Node && !menu.contains(evento.target)) {
        alCerrar();
      }
    };
    const alPulsarTecla = (evento: KeyboardEvent): void => {
      if (evento.key === "Escape") {
        alCerrar();
      } else if (evento.key === "ArrowDown" || evento.key === "ArrowUp") {
        evento.preventDefault();
        const botones = Array.from(menu?.querySelectorAll("button") ?? []);
        const actual = botones.indexOf(document.activeElement as HTMLButtonElement);
        const paso = evento.key === "ArrowDown" ? 1 : -1;
        const siguiente = (actual + paso + botones.length) % botones.length;
        botones[siguiente]?.focus();
      }
    };

    window.addEventListener("mousedown", alPulsarRaton);
    window.addEventListener("keydown", alPulsarTecla);

    return () => {
      window.removeEventListener("mousedown", alPulsarRaton);
      window.removeEventListener("keydown", alPulsarTecla);
    };
  });

  function elegir(id: string): void {
    alElegir(id);
    alCerrar();
  }
</script>

<div
  bind:this={menu}
  class="menu"
  role="menu"
  tabindex="-1"
  style:left="{izquierda}px"
  style:top="{arriba}px"
  style:width="{ancho}px"
>
  {#each items as item (item.id)}
    <button
      type="button"
      role="menuitem"
      class:peligro={item.peligro}
      onclick={() => elegir(item.id)}
    >
      {item.etiqueta}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    display: flex;
    flex-direction: column;
    padding: 6px;
    border: 1px solid var(--borde-fuerte);
    border-radius: var(--radio);
    background: var(--fondo-2);
    box-shadow: 0 18px 40px -18px rgba(0, 0, 0, 0.95);
    animation: abrir var(--transicion);
  }

  @keyframes abrir {
    from {
      opacity: 0;
      transform: scale(0.98);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  button {
    display: flex;
    align-items: center;
    min-height: 28px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radio-chico);
    background: transparent;
    color: var(--texto-medio);
    font-size: 0.78rem;
    text-align: left;
  }

  button:hover,
  button:focus-visible {
    background: var(--fondo-3);
    color: var(--texto);
    outline: none;
  }

  button.peligro {
    color: var(--error);
  }

  button.peligro:hover,
  button.peligro:focus-visible {
    background: var(--fondo-3);
    color: var(--error);
  }
</style>
