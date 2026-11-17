<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import { ago } from "./lib/format";
  import type { Settings, Snapshot } from "./lib/types";
  import DeviceCard from "./lib/components/DeviceCard.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import SettingsView from "./lib/components/SettingsView.svelte";

  let snapshot = $state<Snapshot>({ devices: [], updatedAt: 0 });
  let settings = $state<Settings | null>(null);
  let view = $state<"devices" | "settings">("devices");
  let expanded = $state<string | null>(null);
  let refreshing = $state(false);
  let showHidden = $state(false);
  let autostart = $state(false);
  let version = $state("");
  let now = $state(Date.now());
  let content: HTMLElement;

  let threshold = $derived(settings?.lowThreshold ?? 20);
  let visible = $derived(
    snapshot.devices.filter(
      (d) => (showHidden || !settings?.hidden.includes(d.key)) && (d.online || settings?.showOffline !== false),
    ),
  );
  let hiddenCount = $derived(snapshot.devices.filter((d) => settings?.hidden.includes(d.key)).length);
  let online = $derived(visible.filter((d) => d.online).length);

  async function save(next: Settings) {
    settings = await api.saveSettings(next);
  }

  async function refresh() {
    refreshing = true;
    await api.refresh();
    // The engine replies with a "snapshot" event; stop the spinner then or after 8 s.
    setTimeout(() => (refreshing = false), 8000);
  }

  function toggleHidden(key: string) {
    if (!settings) return;
    const hidden = settings.hidden.includes(key) ? settings.hidden.filter((k) => k !== key) : [...settings.hidden, key];
    save({ ...settings, hidden });
  }

  function rename(key: string, name: string) {
    if (!settings) return;
    const aliases = settings.aliases.filter(([k]) => k !== key);
    aliases.push([key, name]);
    save({ ...settings, aliases });
  }

  onMount(() => {
    const unlisten = api.onSnapshot((s) => {
      snapshot = s;
      refreshing = false;
    });
    // A click on a device in the taskbar widget opens the flyout with that device expanded.
    const focus = (key: string | null) => {
      if (!key) return;
      view = "devices";
      expanded = key;
    };
    const unfocus = api.onFocus((key) => {
      focus(key);
      api.takeFocus();
    });
    api.takeFocus().then(focus);
    api.snapshot().then((s) => (snapshot = s));
    api.settings().then((s) => (settings = s));
    api.autostart().then((a) => (autostart = a));
    api.version().then((v) => (version = v));

    const tick = setInterval(() => (now = Date.now()), 15_000);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (view === "settings") view = "devices";
        else api.hide();
      }
    };
    window.addEventListener("keydown", onKey);
    // Size the native window to the content.
    const ro = new ResizeObserver(() => api.fit(Math.ceil(content.getBoundingClientRect().height)));
    ro.observe(content);
    return () => {
      unlisten.then((f) => f());
      unfocus.then((f) => f());
      clearInterval(tick);
      window.removeEventListener("keydown", onKey);
      ro.disconnect();
    };
  });
</script>

<main bind:this={content}>
  <header>
    {#if view === "settings"}
      <button class="icon-btn" onclick={() => (view = "devices")} aria-label="Back"><Icon name="back" /></button>
      <h1>Settings</h1>
    {:else}
      <div class="brand">
        <span class="logo" aria-hidden="true"></span>
        <div>
          <h1>Juicebar</h1>
          <p>
            {#if snapshot.updatedAt === 0}
              Scanning devices…
            {:else}
              {online} connected · updated {ago(snapshot.updatedAt, now)}
            {/if}
          </p>
        </div>
      </div>
      <div class="tools">
        <button class="icon-btn" class:spin={refreshing} onclick={refresh} aria-label="Refresh" title="Refresh">
          <Icon name="refresh" />
        </button>
        <button class="icon-btn" onclick={() => (view = "settings")} aria-label="Settings" title="Settings">
          <Icon name="gear" />
        </button>
      </div>
    {/if}
  </header>

  <div class="body">
    {#if view === "settings" && settings}
      <SettingsView
        {settings}
        devices={snapshot.devices}
        {autostart}
        {version}
        onsave={save}
        onautostart={async (on) => (autostart = await api.setAutostart(on))}
        onquit={() => api.quit()}
      />
    {:else if visible.length}
      <div class="list">
        {#each visible as device (device.key)}
          <DeviceCard
            {device}
            {threshold}
            hidden={settings?.hidden.includes(device.key)}
            expanded={expanded === device.key}
            ontoggle={() => (expanded = expanded === device.key ? null : device.key)}
            onhide={() => toggleHidden(device.key)}
            onrename={(name) => rename(device.key, name)}
            onforget={() => api.forget(device.key)}
          />
        {/each}
      </div>
    {:else if snapshot.updatedAt === 0}
      <div class="skeleton" aria-busy="true">
        {#each [0, 1, 2] as i}<div class="ghost" style:animation-delay="{i * 120}ms"></div>{/each}
      </div>
    {:else}
      <div class="empty">
        <div class="empty-icon"><Icon name="search" size={26} /></div>
        <h2>No wireless devices found</h2>
        <p>Turn your device on and make sure its receiver is plugged in, or that it's connected over Bluetooth.</p>
        <button class="primary" onclick={refresh}>Scan again</button>
      </div>
    {/if}

    {#if view === "devices" && hiddenCount > 0}
      <button class="hidden-toggle" onclick={() => (showHidden = !showHidden)}>
        {showHidden ? "Hide" : "Show"} {hiddenCount} hidden device{hiddenCount === 1 ? "" : "s"}
      </button>
    {/if}
  </div>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    max-height: 640px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 14px 10px 16px;
    -webkit-app-region: drag;
  }
  header :global(button) {
    -webkit-app-region: no-drag;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }
  .logo {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background:
      linear-gradient(var(--logo-fill), var(--logo-fill)) 7px 10px / 11px 8px no-repeat,
      linear-gradient(var(--on-accent), var(--on-accent)) 20px 12px / 2px 4px no-repeat,
      linear-gradient(135deg, #34d399, #16a34a);
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.15);
    position: relative;
  }
  .logo::after {
    content: "";
    position: absolute;
    left: 5px;
    top: 8px;
    width: 15px;
    height: 12px;
    border: 1.5px solid var(--on-accent);
    border-radius: 3px;
    box-sizing: border-box;
  }
  h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  header > h1 {
    flex: 1;
  }
  .brand p {
    margin: 1px 0 0;
    font-size: 11.5px;
    color: var(--text-2);
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .icon-btn {
    all: unset;
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text-2);
  }
  .icon-btn:hover {
    background: var(--control-hover);
    color: var(--text);
  }
  .icon-btn:focus-visible {
    outline: 2px solid var(--focus);
  }
  .spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }

  .body {
    overflow-y: auto;
    padding: 0 10px 12px;
    scrollbar-width: thin;
  }
  .list {
    display: grid;
    gap: 6px;
  }
  .hidden-toggle {
    all: unset;
    display: block;
    margin: 10px auto 0;
    font-size: 12px;
    color: var(--text-2);
    padding: 4px 8px;
    border-radius: 4px;
  }
  .hidden-toggle:hover {
    color: var(--text);
  }

  .skeleton {
    display: grid;
    gap: 6px;
  }
  .ghost {
    height: 62px;
    border-radius: 8px;
    background: var(--card);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .empty {
    text-align: center;
    padding: 18px 20px 10px;
  }
  .empty-icon {
    width: 52px;
    height: 52px;
    margin: 0 auto 12px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--card);
    color: var(--text-2);
  }
  .empty h2 {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 600;
  }
  .empty p {
    margin: 0 auto 14px;
    max-width: 280px;
    font-size: 12.5px;
    color: var(--text-2);
    line-height: 1.45;
  }
  .primary {
    all: unset;
    padding: 6px 16px;
    border-radius: 6px;
    background: var(--focus);
    color: var(--on-accent);
    font-size: 13px;
    font-weight: 500;
  }
  .primary:hover {
    filter: brightness(1.08);
  }
  .primary:focus-visible {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.5;
    }
  }
</style>
