<script lang="ts">
  import Icon from "./Icon.svelte";
  import Sparkline from "./Sparkline.svelte";
  import { api } from "../api";
  import { levelText, sourceLabel, status, tone, transportLabel } from "../format";
  import type { Device, Sample } from "../types";

  let {
    device,
    threshold,
    expanded,
    hidden = false,
    ontoggle,
    onhide,
    onrename,
    onforget,
  }: {
    device: Device;
    threshold: number;
    expanded: boolean;
    hidden?: boolean;
    ontoggle: () => void;
    onhide: () => void;
    onrename: (name: string) => void;
    onforget: () => void;
  } = $props();

  let t = $derived(tone(device, threshold));
  let charging = $derived(device.charge === "charging");
  let cells = $derived(device.batteries.length > 1 ? device.batteries : []);
  let history = $state<Sample[]>([]);
  let renaming = $state(false);
  let draft = $state("");

  $effect(() => {
    if (expanded) {
      const key = device.key;
      api.history(key).then((h) => {
        if (key === device.key) history = h;
      });
    }
  });

  function startRename(e: Event) {
    e.stopPropagation();
    draft = device.name;
    renaming = true;
  }

  function commitRename() {
    renaming = false;
    const name = draft.trim();
    if (name && name !== device.name) onrename(name);
  }
</script>

<article class="card tone-{t}" class:open={expanded} class:offline={!device.online}>
  <button class="row" onclick={ontoggle} aria-expanded={expanded}>
    <span class="glyph">
      <Icon name={device.kind} size={20} />
      {#if charging}<span class="bolt"><Icon name="bolt" size={11} stroke={2.2} /></span>{/if}
    </span>
    <span class="text">
      <span class="name">{device.name}</span>
      <span class="sub">
        {#if !device.online}<Icon name="sleep" size={11} />{/if}
        {status(device)}
      </span>
    </span>
    <span class="level">
      <span class="pct">{levelText(device.level, device.step)}</span>
      <span class="bar" aria-hidden="true"><span class="fill" style:width="{device.level}%"></span></span>
    </span>
  </button>

  {#if cells.length}
    <div class="cells">
      {#each cells as c}
        <div class="cell" class:case={c.label === "Case"}>
          <span class="cell-label">{c.label}</span>
          <span class="cell-bar"><span class="fill" style:width="{c.level}%"></span></span>
          <span class="cell-pct">{c.level}%{#if c.charge === "charging"} <Icon name="bolt" size={10} />{/if}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if expanded}
    <div class="details">
      <Sparkline samples={history} />
      <dl>
        <div><dt>Connection</dt><dd>{transportLabel[device.transport]}</dd></div>
        <div><dt>Source</dt><dd>{sourceLabel[device.source] ?? device.source}</dd></div>
        {#if device.estimate}
          <div>
            <dt>Rate</dt>
            <dd>{device.estimate.ratePerHour > 0 ? "+" : ""}{device.estimate.ratePerHour.toFixed(1)}%/h</dd>
          </div>
        {/if}
        {#if device.vendor}<div><dt>Vendor</dt><dd>{device.vendor}</dd></div>{/if}
      </dl>
      {#if renaming}
        <form
          class="rename"
          onsubmit={(e) => {
            e.preventDefault();
            commitRename();
          }}
        >
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={draft} autofocus maxlength="40" onblur={commitRename} aria-label="Device name" />
        </form>
      {:else}
        <div class="actions">
          <button class="chip" onclick={startRename}><Icon name="edit" size={14} />Rename</button>
          <button class="chip" onclick={onhide}><Icon name={hidden ? "show" : "hide"} size={14} />{hidden ? "Show" : "Hide"}</button>
          {#if !device.online}
            <button class="chip danger" onclick={onforget}><Icon name="trash" size={14} />Forget</button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</article>

<style>
  .card {
    --accent: var(--ok);
    border-radius: 8px;
    background: var(--card);
    border: 1px solid var(--card-border);
    transition: background 120ms ease;
  }
  .card:hover {
    background: var(--card-hover);
  }
  .tone-warn {
    --accent: var(--warn);
  }
  .tone-low {
    --accent: var(--low);
  }
  .tone-charging {
    --accent: var(--charging);
  }
  .tone-idle {
    --accent: var(--text-3);
  }
  .offline .name,
  .offline .pct {
    opacity: 0.6;
  }

  .row {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: grid;
    grid-template-columns: 40px 1fr auto;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    cursor: default;
  }
  .row:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
    border-radius: 8px;
  }
  .glyph {
    position: relative;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .bolt {
    position: absolute;
    right: -3px;
    bottom: -3px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--charging);
    color: var(--on-accent);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-weight: 600;
    font-size: 13.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .level {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }
  .pct {
    font-size: 18px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
    line-height: 1;
  }
  .tone-low .pct {
    color: var(--low);
  }
  .bar,
  .cell-bar {
    position: relative;
    width: 56px;
    height: 4px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background: var(--accent);
    transition: width 400ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }

  .cells {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    padding: 0 12px 10px 64px;
  }
  .cell {
    display: grid;
    gap: 3px;
    font-size: 11px;
    color: var(--text-2);
  }
  .cell-bar {
    width: 100%;
  }
  .cell.case .fill {
    background: var(--text-3);
  }
  .cell-pct {
    color: var(--text);
    font-variant-numeric: tabular-nums;
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .details {
    padding: 2px 12px 12px;
    display: grid;
    gap: 10px;
    animation: reveal 160ms ease-out;
  }
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 12px;
  }
  dl div {
    display: flex;
    flex-direction: column;
  }
  dt {
    font-size: 11px;
    color: var(--text-3);
  }
  dd {
    margin: 0;
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .chip {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 10px;
    border-radius: 6px;
    font-size: 12px;
    background: var(--control);
    border: 1px solid var(--card-border);
  }
  .chip:hover {
    background: var(--control-hover);
  }
  .chip:focus-visible {
    outline: 2px solid var(--focus);
  }
  .chip.danger {
    color: var(--low);
  }
  .rename input {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
    font-size: 13px;
    padding: 6px 10px;
    border-radius: 6px;
    border: 1px solid var(--card-border);
    border-bottom: 2px solid var(--focus);
    background: var(--control);
    color: var(--text);
    outline: none;
  }

  @keyframes reveal {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
</style>
