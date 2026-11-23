<script lang="ts">
  import type { Toast } from "./toast";

  let {
    toasts,
    alCerrar,
  }: {
    toasts: Toast[];
    alCerrar: (id: number) => void;
  } = $props();
</script>

<div class="toasts" aria-live="polite" aria-atomic="false">
  {#each toasts as toast (toast.id)}
    <div class="toast" class:error={toast.tono === "error"} class:aviso={toast.tono === "aviso"}>
      <span class="texto">{toast.texto}</span>
      <button type="button" class="cerrar" aria-label="Cerrar" onclick={() => alCerrar(toast.id)}>
        ×
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--esp-2);
    bottom: calc(var(--barra-estado) + var(--esp-2));
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: var(--esp-1);
    width: min(380px, calc(100vw - var(--barra-lateral) - 2 * var(--esp-2)));
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--esp-1);
    padding: 10px 12px;
    border: 1px solid var(--borde-fuerte);
    border-left: 3px solid var(--texto);
    border-radius: var(--radio-chico);
    background: var(--fondo-2);
    color: var(--texto);
    font-size: 0.8rem;
    line-height: 1.4;
    pointer-events: auto;
    animation: entrar var(--transicion);
    box-shadow: 0 12px 32px -20px rgba(0, 0, 0, 0.9);
  }

  .toast.error {
    border-left-color: var(--error);
  }

  .toast.aviso {
    border-left-color: var(--aviso);
  }

  @keyframes entrar {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .texto {
    flex: 1;
    user-select: text;
  }

  .cerrar {
    border: none;
    background: transparent;
    color: var(--texto-tenue);
    font-size: 1rem;
    line-height: 1;
    padding: 0 2px;
  }

  .cerrar:hover {
    color: var(--texto);
  }
</style>
