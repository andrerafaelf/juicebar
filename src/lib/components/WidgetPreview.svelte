<script lang="ts" module>
  import type { DeviceKind } from "../types";

  export interface PreviewItem {
    kind: DeviceKind;
    name: string;
    level: number;
    low: boolean;
    charging: boolean;
  }
</script>

<script lang="ts">
  // A scaled-down replica of the native taskbar widget, used to pick its style.
  import Icon from "./Icon.svelte";

  let { style, items }: { style: "detailed" | "compact"; items: PreviewItem[] } = $props();
</script>

<div class="strip" class:compact={style === "compact"} aria-hidden="true">
  {#each items as it (it.name)}
    <div class="item" class:low={it.low}>
      <Icon name={it.kind} size={12} stroke={1.9} />
      {#if style === "compact"}
        <span class="pct">{it.level}%</span>
      {:else}
        <div class="lines">
          <span class="name">{it.name}</span>
          <span class="meter">
            <span class="gauge"><span style:width="{it.level}%"></span></span>
            <span class="pct">{it.level}%</span>
          </span>
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 34px;
    padding: 0 9px;
    border-radius: 6px;
    background: var(--taskbar);
    color: var(--taskbar-text);
    overflow: hidden;
    white-space: nowrap;
    box-shadow: inset 0 1px 0 var(--taskbar-edge);
    /* A cropped slice of the taskbar: fade out the right edge. */
    mask-image: linear-gradient(to right, #000 calc(100% - 16px), transparent);
  }
  .strip.compact {
    gap: 9px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    flex: none;
  }
  .lines {
    display: grid;
    gap: 1px;
    min-width: 0;
  }
  .name {
    font-size: 8.5px;
    line-height: 1.1;
    max-width: 46px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .gauge {
    position: relative;
    width: 11px;
    height: 5px;
    border-radius: 1.5px;
    box-shadow: 0 0 0 0.75px color-mix(in srgb, currentColor 55%, transparent);
    overflow: hidden;
  }
  .gauge span {
    position: absolute;
    inset: 1px auto 1px 1px;
    max-width: calc(100% - 2px);
    border-radius: 0.5px;
    background: var(--ok);
  }
  .pct {
    font-size: 8.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .compact .pct {
    font-size: 10px;
    font-weight: 500;
  }
  .low .pct {
    color: var(--low);
  }
  .low .gauge span {
    background: var(--low);
  }
</style>
