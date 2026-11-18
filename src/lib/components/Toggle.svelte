<script lang="ts">
  // Windows 11 toggle switch: the knob grows on hover and stretches while pressed.
  let { checked, label, onchange }: { checked: boolean; label: string; onchange: (value: boolean) => void } = $props();
</script>

<button class="toggle" class:on={checked} role="switch" aria-checked={checked} aria-label={label} onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .toggle {
    all: unset;
    flex: none;
    position: relative;
    width: 40px;
    height: 20px;
    box-sizing: border-box;
    border-radius: 10px;
    border: 1px solid var(--text-2);
    background: var(--control);
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }
  .toggle:hover {
    background: var(--control-hover);
  }
  .knob {
    position: absolute;
    top: 50%;
    left: 4px;
    width: 12px;
    height: 12px;
    border-radius: 6px;
    background: var(--text-2);
    transform: translateY(-50%);
    transition:
      left 170ms cubic-bezier(0.2, 0.9, 0.3, 1),
      width 120ms ease,
      height 120ms ease,
      background 150ms ease;
  }
  .toggle:hover .knob {
    width: 14px;
    height: 14px;
    left: 3px;
  }
  .toggle:active .knob {
    width: 17px;
  }
  .toggle.on {
    background: var(--focus);
    border-color: var(--focus);
  }
  .toggle.on:hover {
    filter: brightness(1.06);
  }
  .toggle.on .knob {
    background: var(--on-accent);
    left: 24px;
  }
  .toggle.on:hover .knob {
    left: 22px;
  }
  .toggle.on:active .knob {
    left: 19px;
  }
  .toggle:focus-visible {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }
</style>
