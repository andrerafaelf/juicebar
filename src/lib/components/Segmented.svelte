<script lang="ts" generics="T extends string | number">
  // Equal-width segmented control with a sliding selection pill.
  let {
    value,
    options,
    label,
    disabled = false,
    onchange,
  }: {
    value: T;
    options: [T, string][];
    label: string;
    disabled?: boolean;
    onchange: (value: T) => void;
  } = $props();

  let index = $derived(Math.max(0, options.findIndex(([v]) => v === value)));
</script>

<div class="segmented" role="radiogroup" aria-label={label} style:--n={options.length} style:--i={index}>
  <span class="pill" aria-hidden="true"></span>
  {#each options as [v, text] (v)}
    <button role="radio" aria-checked={v === value} class:on={v === value} {disabled} onclick={() => v !== value && onchange(v)}>
      {text}
    </button>
  {/each}
</div>

<style>
  /* Equal columns as wide as the longest label (at least 40px). */
  .segmented {
    flex: none;
    position: relative;
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(40px, 1fr);
    padding: 2px;
    border-radius: 6px;
    background: var(--control);
    border: 1px solid var(--card-border);
  }
  .pill {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 2px;
    width: calc((100% - 4px) / var(--n));
    border-radius: 4px;
    background: var(--card-hover);
    box-shadow:
      0 0 0 1px var(--card-border),
      0 1px 2px rgb(0 0 0 / 0.18);
    transform: translateX(calc(var(--i) * 100%));
    transition: transform 180ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  button {
    all: unset;
    position: relative;
    text-align: center;
    padding: 0 9px;
    font-size: 12px;
    line-height: 24px;
    border-radius: 4px;
    color: var(--text-2);
    transition: color 120ms ease;
  }
  button:not(:disabled):hover {
    color: var(--text);
  }
  button.on {
    color: var(--text);
    font-weight: 500;
  }
  button:focus-visible {
    outline: 2px solid var(--focus);
  }
</style>
