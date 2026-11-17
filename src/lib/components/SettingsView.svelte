<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { Device, Settings, TrayMode } from "../types";

  let {
    settings,
    devices,
    autostart,
    version,
    onsave,
    onautostart,
    onquit,
  }: {
    settings: Settings;
    devices: Device[];
    autostart: boolean;
    version: string;
    onsave: (s: Settings) => void;
    onautostart: (enabled: boolean) => void;
    onquit: () => void;
  } = $props();

  const intervals = [
    [30, "30 seconds"],
    [60, "1 minute"],
    [120, "2 minutes"],
    [300, "5 minutes"],
  ] as const;

  function patch(p: Partial<Settings>) {
    onsave({ ...settings, ...p });
  }

  let hiddenDevices = $derived(
    settings.hidden.map((key) => ({ key, name: devices.find((d) => d.key === key)?.name ?? key })),
  );
  let hscPath = $state("");
  $effect(() => {
    hscPath = settings.headsetcontrolPath ?? "";
  });
</script>

<div class="settings">
  <section>
    <h3>General</h3>
    <label class="row">
      <span><strong>Start with Windows</strong><small>Runs quietly in the notification area</small></span>
      <input type="checkbox" class="switch" checked={autostart} onchange={(e) => onautostart(e.currentTarget.checked)} />
    </label>
    <label class="row">
      <span><strong>Taskbar widget</strong><small>Levels next to the clock, inside the taskbar</small></span>
      <input type="checkbox" class="switch" checked={settings.taskbarWidget} onchange={(e) => patch({ taskbarWidget: e.currentTarget.checked })} />
    </label>
    {#if settings.taskbarWidget}
      <div class="row">
        <span><strong>Widget style</strong><small>Name and gauge, or just the percentage</small></span>
        <div class="segmented" role="radiogroup" aria-label="Widget style">
          {#each [["detailed", "Detailed"], ["compact", "Compact"]] as [value, label]}
            <button
              role="radio"
              aria-checked={settings.widgetStyle === value}
              class:on={settings.widgetStyle === value}
              onclick={() => patch({ widgetStyle: value as Settings["widgetStyle"] })}>{label}</button
            >
          {/each}
        </div>
      </div>
    {/if}
    <div class="row">
      <span><strong>Tray icon</strong><small>{settings.taskbarWidget ? "Shown anyway if the widget can't attach" : "Lowest battery, or one per device"}</small></span>
      <div class="segmented" role="radiogroup" aria-label="Tray icon mode">
        {#each [["off", "Off"], ["single", "Lowest"], ["per-device", "Each"]] as [value, label]}
          <button
            role="radio"
            aria-checked={settings.trayMode === value}
            class:on={settings.trayMode === value}
            onclick={() => patch({ trayMode: value as TrayMode })}>{label}</button
          >
        {/each}
      </div>
    </div>
    <label class="row">
      <span><strong>Show asleep devices</strong><small>Keep last known level for 24 h</small></span>
      <input type="checkbox" class="switch" checked={settings.showOffline} onchange={(e) => patch({ showOffline: e.currentTarget.checked })} />
    </label>
    <label class="row">
      <span><strong>Check every</strong></span>
      <select value={settings.pollSeconds} onchange={(e) => patch({ pollSeconds: Number(e.currentTarget.value) })}>
        {#each intervals as [v, l]}<option value={v}>{l}</option>{/each}
      </select>
    </label>
  </section>

  <section>
    <h3>Notifications</h3>
    <label class="row">
      <span><strong>Low battery</strong><small>Alert at {settings.lowThreshold}% or below</small></span>
      <input type="checkbox" class="switch" checked={settings.notifyLow} onchange={(e) => patch({ notifyLow: e.currentTarget.checked })} />
    </label>
    <div class="row slider">
      <input
        type="range"
        min="5"
        max="50"
        step="5"
        value={settings.lowThreshold}
        aria-label="Low battery threshold"
        oninput={(e) => patch({ lowThreshold: Number(e.currentTarget.value) })}
      />
      <span class="value">{settings.lowThreshold}%</span>
    </div>
    <label class="row">
      <span><strong>Fully charged</strong><small>Tell me when I can unplug</small></span>
      <input type="checkbox" class="switch" checked={settings.notifyFull} onchange={(e) => patch({ notifyFull: e.currentTarget.checked })} />
    </label>
  </section>

  <section>
    <h3>Advanced</h3>
    <div class="row column">
      <span><strong>HeadsetControl</strong><small>Adds SteelSeries, Corsair, HyperX and more headsets. Leave empty to search PATH.</small></span>
      <input
        type="text"
        placeholder="C:\Tools\headsetcontrol.exe"
        bind:value={hscPath}
        onchange={() => patch({ headsetcontrolPath: hscPath.trim() || null })}
      />
    </div>
    {#if hiddenDevices.length}
      <div class="row column">
        <span><strong>Hidden devices</strong></span>
        <ul>
          {#each hiddenDevices as h}
            <li>
              <span>{h.name}</span>
              <button class="link" onclick={() => patch({ hidden: settings.hidden.filter((k) => k !== h.key) })}>
                <Icon name="show" size={14} /> Show
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </section>

  <footer>
    <span>Juicebar {version}</span>
    <button class="quit" onclick={onquit}><Icon name="power" size={14} /> Quit</button>
  </footer>
</div>

<style>
  .settings {
    display: grid;
    gap: 14px;
  }
  section {
    display: grid;
    gap: 2px;
  }
  h3 {
    margin: 0 0 4px 2px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    background: var(--card);
    border: 1px solid var(--card-border);
    border-radius: 6px;
  }
  .row.column {
    flex-direction: column;
    align-items: stretch;
  }
  .row.slider {
    padding-top: 6px;
    padding-bottom: 8px;
  }
  .row span:first-child {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  strong {
    font-weight: 500;
    font-size: 13px;
  }
  small {
    font-size: 11.5px;
    color: var(--text-2);
  }
  .value {
    font-variant-numeric: tabular-nums;
    min-width: 34px;
    text-align: right;
    font-size: 12.5px;
  }

  .switch {
    appearance: none;
    flex: none;
    width: 40px;
    height: 20px;
    border-radius: 10px;
    border: 1px solid var(--text-2);
    position: relative;
    margin: 0;
    transition: background 120ms;
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-2);
    transition: transform 140ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .switch:checked {
    background: var(--focus);
    border-color: var(--focus);
  }
  .switch:checked::after {
    transform: translateX(20px);
    background: var(--on-accent);
  }
  .switch:focus-visible,
  select:focus-visible,
  input[type="range"]:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }

  input[type="range"] {
    flex: 1;
    accent-color: var(--focus);
  }
  select,
  input[type="text"] {
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    background: var(--control);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    padding: 5px 8px;
  }
  input[type="text"] {
    border-bottom: 1px solid var(--text-3);
    outline: none;
  }
  input[type="text"]:focus {
    border-bottom: 2px solid var(--focus);
  }
  option {
    background: var(--surface-solid);
  }

  .segmented {
    display: inline-flex;
    padding: 2px;
    border-radius: 6px;
    background: var(--control);
    border: 1px solid var(--card-border);
  }
  .segmented button {
    all: unset;
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 4px;
    color: var(--text-2);
  }
  .segmented button.on {
    background: var(--card-hover);
    color: var(--text);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }
  .segmented button:focus-visible {
    outline: 2px solid var(--focus);
  }

  ul {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12.5px;
  }
  .link,
  .quit {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--focus);
    padding: 2px 4px;
    border-radius: 4px;
  }
  .link:focus-visible,
  .quit:focus-visible {
    outline: 2px solid var(--focus);
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11.5px;
    color: var(--text-3);
    padding: 0 2px;
  }
  .quit {
    color: var(--text-2);
  }
  .quit:hover {
    color: var(--low);
  }
</style>
