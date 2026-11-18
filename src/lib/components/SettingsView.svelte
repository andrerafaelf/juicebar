<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "./Icon.svelte";
  import Segmented from "./Segmented.svelte";
  import Toggle from "./Toggle.svelte";
  import WidgetPreview, { type PreviewItem } from "./WidgetPreview.svelte";
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

  function patch(p: Partial<Settings>) {
    onsave({ ...settings, ...p });
  }

  let advanced = $state(false);
  let hscPath = $state("");
  $effect.pre(() => {
    hscPath = settings.headsetcontrolPath ?? "";
  });

  // The style picker previews the user's own devices (or a sample set before any is found).
  const sample: PreviewItem[] = [
    { kind: "mouse", name: "Mouse", level: 72, low: false, charging: false },
    { kind: "keyboard", name: "Keyboard", level: 88, low: false, charging: false },
    { kind: "headset", name: "Headset", level: 14, low: true, charging: false },
  ];
  let preview = $derived.by((): PreviewItem[] => {
    const own = devices
      .filter((d) => d.online && !settings.hidden.includes(d.key))
      .slice(0, 3)
      .map((d) => ({
        kind: d.kind,
        name: d.name,
        level: d.level,
        low: d.level <= settings.lowThreshold,
        charging: d.charge === "charging",
      }));
    return own.length ? own : sample;
  });

  const styles = [
    ["detailed", "Detailed"],
    ["compact", "Minimal"],
  ] as const;
  const thresholds = [10, 15, 20, 30];
  let thresholdOptions = $derived(
    [...new Set([...thresholds, settings.lowThreshold])]
      .sort((a, b) => a - b)
      .map((v): [number, string] => [v, `${v}%`]),
  );

  let hiddenDevices = $derived(
    settings.hidden.map((key) => ({ key, name: devices.find((d) => d.key === key)?.name ?? key })),
  );
  let hasG435 = $derived(devices.some((d) => d.source === "g435") || settings.g435Minutes !== 10);
</script>

<div class="settings">
  <section>
    <h3>Taskbar</h3>
    <div class="group">
      <div class="row">
        <span class="icon"><Icon name="taskbar" /></span>
        <div class="label"><strong>Show levels in the taskbar</strong></div>
        <Toggle label="Taskbar widget" checked={settings.taskbarWidget} onchange={(v) => patch({ taskbarWidget: v })} />
      </div>
      <div class="styles" class:disabled={!settings.taskbarWidget} role="radiogroup" aria-label="Widget style">
        {#each styles as [value, label] (value)}
          <button
            class="style"
            class:on={settings.widgetStyle === value}
            role="radio"
            aria-checked={settings.widgetStyle === value}
            disabled={!settings.taskbarWidget}
            onclick={() => patch({ widgetStyle: value })}
          >
            <WidgetPreview style={value} items={value === "compact" ? preview : preview.slice(0, 2)} />
            <span class="caption">
              {label}
              <span class="tick"><Icon name="check" size={11} stroke={2.6} /></span>
            </span>
          </button>
        {/each}
      </div>
    </div>
  </section>

  <section>
    <h3>Alerts</h3>
    <div class="group">
      <div class="row">
        <span class="icon"><Icon name="bell" /></span>
        <div class="label">
          <strong>Low battery</strong>
          <small>Notify once when a device drops this low</small>
        </div>
        <Toggle label="Low battery alert" checked={settings.notifyLow} onchange={(v) => patch({ notifyLow: v })} />
      </div>
      <div class="row sub" class:disabled={!settings.notifyLow}>
        <span class="icon"></span>
        <div class="label"><small>Alert at</small></div>
        <Segmented
          label="Low battery threshold"
          disabled={!settings.notifyLow}
          value={settings.lowThreshold}
          options={thresholdOptions}
          onchange={(v) => patch({ lowThreshold: v })}
        />
      </div>
      <div class="row">
        <span class="icon"><Icon name="bolt" /></span>
        <div class="label">
          <strong>Fully charged</strong>
          <small>Tell me when I can unplug</small>
        </div>
        <Toggle label="Fully charged alert" checked={settings.notifyFull} onchange={(v) => patch({ notifyFull: v })} />
      </div>
    </div>
  </section>

  <section>
    <h3>General</h3>
    <div class="group">
      <div class="row">
        <span class="icon"><Icon name="clock" /></span>
        <div class="label"><strong>Check every</strong></div>
        <Segmented
          label="Check every"
          value={settings.pollSeconds}
          options={[
            [30, "30s"],
            [60, "1m"],
            [120, "2m"],
            [300, "5m"],
          ]}
          onchange={(v) => patch({ pollSeconds: v })}
        />
      </div>
      <div class="row">
        <span class="icon"><Icon name="sleep" /></span>
        <div class="label">
          <strong>Keep asleep devices</strong>
          <small>Dimmed, with their last level</small>
        </div>
        <Toggle label="Keep asleep devices" checked={settings.showOffline} onchange={(v) => patch({ showOffline: v })} />
      </div>
      <div class="row">
        <span class="icon"><Icon name="login" /></span>
        <div class="label"><strong>Start with Windows</strong></div>
        <Toggle label="Start with Windows" checked={autostart} onchange={onautostart} />
      </div>
      <button class="row disclosure" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>
        <span class="icon"><Icon name="sliders" /></span>
        <div class="label"><strong>Advanced</strong></div>
        <span class="chevron" class:open={advanced}><Icon name="chevronDown" size={16} /></span>
      </button>
      {#if advanced}
        <div class="advanced" transition:slide={{ duration: 200, easing: cubicOut }}>
          <div class="row">
            <span class="icon"><Icon name="tray" /></span>
            <div class="label">
              <strong>Tray icon</strong>
              <small>{settings.taskbarWidget ? "Fallback for the widget" : "Lowest, or one per device"}</small>
            </div>
            <Segmented
              label="Tray icon"
              value={settings.trayMode}
              options={[
                ["off", "Off"],
                ["single", "Lowest"],
                ["per-device", "Each"],
              ]}
              onchange={(v: TrayMode) => patch({ trayMode: v })}
            />
          </div>
          {#if hasG435}
            <div class="row">
              <span class="icon"><Icon name="headset" /></span>
              <div class="label">
                <strong>Logitech G435</strong>
                <small>Checks only while the headset is silent</small>
              </div>
              <Segmented
                label="Logitech G435 check interval"
                value={settings.g435Minutes}
                options={[
                  [5, "5m"],
                  [10, "10m"],
                  [30, "30m"],
                  [0, "Off"],
                ]}
                onchange={(v) => patch({ g435Minutes: v })}
              />
            </div>
          {/if}
          <div class="row stacked">
            <span class="icon"><Icon name="code" /></span>
            <div class="label">
              <strong>HeadsetControl</strong>
              <small>Adds SteelSeries, Corsair, HyperX and more</small>
              <input
                type="text"
                spellcheck="false"
                placeholder="Search PATH"
                bind:value={hscPath}
                onchange={() => patch({ headsetcontrolPath: hscPath.trim() || null })}
              />
            </div>
          </div>
          {#if hiddenDevices.length}
            <div class="row stacked">
              <span class="icon"><Icon name="hide" /></span>
              <div class="label">
                <strong>Hidden devices</strong>
                <ul>
                  {#each hiddenDevices as h (h.key)}
                    <li>
                      <span>{h.name}</span>
                      <button
                        class="link"
                        onclick={() => patch({ hidden: settings.hidden.filter((k) => k !== h.key) })}>Show</button
                      >
                    </li>
                  {/each}
                </ul>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </section>

  <footer>
    <span>Juicebar {version}</span>
    <button class="quit" onclick={onquit}><Icon name="power" size={13} /> Quit</button>
  </footer>
</div>

<style>
  .settings {
    display: grid;
    gap: 18px;
    padding-top: 2px;
  }
  section {
    display: grid;
    gap: 7px;
  }
  h3 {
    margin: 0 0 0 2px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .group {
    background: var(--card);
    border: 1px solid var(--card-border);
    border-radius: 8px;
    overflow: hidden;
  }

  /* Rows: fixed icon column, label, control on the right. */
  .row {
    display: grid;
    grid-template-columns: 20px 1fr auto;
    align-items: center;
    column-gap: 12px;
    min-height: 48px;
    padding: 7px 12px;
    box-sizing: border-box;
    transition: opacity 150ms ease;
  }
  .row + .row,
  .advanced > .row,
  .row + .advanced {
    border-top: 1px solid var(--line-soft);
  }
  .row.sub {
    border-top: none;
    min-height: 0;
    padding-top: 0;
    padding-bottom: 10px;
  }
  .row.stacked {
    align-items: start;
    padding-top: 11px;
    padding-bottom: 12px;
  }
  .row.stacked .icon {
    margin-top: 1px;
  }
  .row.disabled,
  .styles.disabled {
    opacity: 0.4;
  }
  .icon {
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  .label {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  strong {
    font-weight: 500;
    font-size: 13px;
  }
  small {
    font-size: 11.5px;
    color: var(--text-2);
    line-height: 1.3;
  }

  /* Widget style picker. */
  .styles {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    padding: 2px 12px 12px;
    transition: opacity 150ms ease;
  }
  .style {
    all: unset;
    display: grid;
    gap: 7px;
    padding: 6px;
    border-radius: 8px;
    background: var(--control);
    box-shadow: inset 0 0 0 1px var(--card-border);
    transition:
      box-shadow 140ms ease,
      background 140ms ease;
  }
  .style:not(:disabled):hover {
    background: var(--control-hover);
  }
  .style.on {
    box-shadow: inset 0 0 0 2px var(--focus);
  }
  .style:focus-visible {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }
  .caption {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 1px 3px;
    font-size: 12px;
    color: var(--text-2);
  }
  .style.on .caption {
    color: var(--text);
    font-weight: 500;
  }
  .tick {
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    color: var(--on-accent);
    background: var(--focus);
    transform: scale(0);
    transition: transform 160ms cubic-bezier(0.3, 1.4, 0.5, 1);
  }
  .style.on .tick {
    transform: scale(1);
  }

  /* Advanced disclosure. */
  .disclosure {
    all: unset;
    display: grid;
    grid-template-columns: 20px 1fr auto;
    align-items: center;
    column-gap: 12px;
    min-height: 44px;
    padding: 7px 12px;
    box-sizing: border-box;
    width: 100%;
    border-top: 1px solid var(--line-soft);
  }
  .disclosure:hover {
    background: var(--control-hover);
  }
  .disclosure:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .chevron {
    display: grid;
    color: var(--text-2);
    transition: transform 200ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  .chevron.open {
    transform: rotate(180deg);
  }

  input[type="text"] {
    margin-top: 8px;
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    background: var(--control);
    border: 1px solid var(--card-border);
    border-bottom-color: var(--text-3);
    border-radius: 5px;
    padding: 5px 9px;
    outline: none;
  }
  input[type="text"]:focus {
    border-bottom: 2px solid var(--focus);
    padding-bottom: 4px;
  }
  input[type="text"]::placeholder {
    color: var(--text-3);
  }
  ul {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: grid;
    gap: 2px;
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
    gap: 5px;
    font-size: 12px;
    padding: 3px 8px;
    border-radius: 5px;
  }
  .link {
    color: var(--focus);
  }
  .link:hover,
  .quit:hover {
    background: var(--control-hover);
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
    padding: 0 0 0 2px;
  }
  .quit {
    color: var(--text-2);
  }
  .quit:hover {
    color: var(--low);
  }
</style>
