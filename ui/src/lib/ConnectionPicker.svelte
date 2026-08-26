<script lang="ts">
  import {
    DEFAULT_PROFILE,
    brokerLabel,
    type ProfileSummary,
  } from "./types";

  let {
    profiles,
    lastUsedName = null,
    busy = false,
    loaded = false,
    onnew,
    onedit,
    onduplicate,
    ondelete,
    onconnect,
    onsettings,
  }: {
    profiles: ProfileSummary[];
    lastUsedName?: string | null;
    busy?: boolean;
    loaded?: boolean;
    onnew: () => void;
    onedit: (id: string) => void;
    onduplicate: (id: string) => void;
    ondelete: (id: string, name: string) => void;
    onconnect: (id: string) => void;
    onsettings: () => void;
  } = $props();

  let menuId = $state<string | null>(null);
  let menuStyle = $state("");

  const cards = $derived(profiles.length > 0 ? profiles : [DEFAULT_PROFILE]);
  const placeholder = $derived(profiles.length === 0);

  function placeMenu(button: HTMLElement) {
    const rect = button.getBoundingClientRect();
    const gap = 4;
    const estimatedHeight = placeholder ? 40 : 112;
    const dropUp =
      window.innerHeight - rect.bottom - gap < estimatedHeight && rect.top > estimatedHeight;
    menuStyle = dropUp
      ? `bottom:${window.innerHeight - rect.top + gap}px;right:${window.innerWidth - rect.right}px;`
      : `top:${rect.bottom + gap}px;right:${window.innerWidth - rect.right}px;`;
  }

  function toggleMenu(id: string, event: MouseEvent) {
    event.stopPropagation();
    if (menuId === id) {
      closeMenu();
      return;
    }
    placeMenu(event.currentTarget as HTMLElement);
    menuId = id;
  }

  function closeMenu() {
    menuId = null;
    menuStyle = "";
  }

  $effect(() => {
    if (!menuId) {
      return;
    }
    function onPointerDown(event: PointerEvent) {
      const target = event.target;
      if (target instanceof Element && target.closest("[data-profile-menu]")) {
        return;
      }
      closeMenu();
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        closeMenu();
      }
    }
    function onReposition() {
      closeMenu();
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("resize", onReposition);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onReposition);
    };
  });
</script>

<section class="picker">
  <header>
    <h2>Connections</h2>
    <div class="header-actions">
      <button type="button" class="gear" aria-label="Settings" onclick={onsettings}>⚙</button>
      <button type="button" class="new" disabled={busy} onclick={onnew}>+ New</button>
    </div>
  </header>

  {#if !loaded}
    <p class="muted">Loading…</p>
  {:else}
    <ul class="grid" onscroll={closeMenu}>
      {#each cards as profile (profile.id || "default")}
        {@const open = menuId === (profile.id || "default")}
        <li>
          <article class="card" class:placeholder>
            <div class="card-top">
              <div class="titles">
                <h3>{profile.name}</h3>
                <p class="meta">{brokerLabel(profile)}</p>
              </div>
              <div class="menu" data-profile-menu>
                <button
                  type="button"
                  class="icon"
                  aria-haspopup="menu"
                  aria-expanded={open}
                  aria-label="Profile actions"
                  disabled={busy}
                  onclick={(event) => toggleMenu(profile.id || "default", event)}
                >
                  ⋯
                </button>
                {#if open}
                  <ul class="dropdown" role="menu" style={menuStyle}>
                    <li>
                      <button
                        type="button"
                        role="menuitem"
                        onclick={() => {
                          closeMenu();
                          onedit(profile.id);
                        }}
                      >
                        Edit
                      </button>
                    </li>
                    {#if !placeholder}
                      <li>
                        <button
                          type="button"
                          role="menuitem"
                          onclick={() => {
                            closeMenu();
                            onduplicate(profile.id);
                          }}
                        >
                          Duplicate
                        </button>
                      </li>
                      <li>
                        <button
                          type="button"
                          class="danger"
                          role="menuitem"
                          onclick={() => {
                            closeMenu();
                            ondelete(profile.id, profile.name);
                          }}
                        >
                          Delete
                        </button>
                      </li>
                    {/if}
                  </ul>
                {/if}
              </div>
            </div>
            <div class="card-actions">
              <button
                type="button"
                class="connect"
                disabled={busy}
                onclick={() => onconnect(profile.id)}
              >
                Connect
              </button>
            </div>
          </article>
        </li>
      {/each}
    </ul>
  {/if}

  {#if lastUsedName}
    <p class="last-used">Last used · {lastUsedName}</p>
  {/if}
</section>

<style>
  .picker {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 56rem;
    width: 100%;
    margin: 0 auto;
    padding: var(--space-5);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  h2 {
    font-size: 1.15rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .new,
  .gear {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
  }

  .new {
    padding: 0.4rem 0.75rem;
    font-weight: 600;
  }

  .gear {
    width: 2.1rem;
    height: 2.1rem;
    color: var(--fg-muted);
  }

  .grid,
  .muted {
    flex: 1;
    min-height: 0;
  }

  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    align-content: start;
    grid-template-columns: repeat(auto-fill, minmax(16.5rem, 1fr));
    gap: var(--space-3);
    overflow: auto;
  }

  .card {
    min-height: 8.25rem;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }

  .card.placeholder {
    border-style: dashed;
    box-shadow: none;
  }

  .card-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .titles {
    min-width: 0;
  }

  h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 650;
  }

  .meta {
    margin: 0.15rem 0 0;
    color: var(--fg-muted);
    font-family: var(--mono);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .menu {
    position: relative;
  }

  .icon {
    width: 1.8rem;
    height: 1.8rem;
    border: 1px solid transparent;
    background: transparent;
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    line-height: 1;
  }

  .icon:hover,
  .icon[aria-expanded="true"] {
    background: var(--bg-hover);
    border-color: var(--border);
    color: var(--fg);
  }

  .dropdown {
    position: fixed;
    z-index: 5;
    list-style: none;
    margin: 0;
    padding: var(--space-1);
    min-width: 8.5rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow);
  }

  .dropdown button {
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    border-radius: 4px;
    padding: 0.4rem 0.55rem;
  }

  .dropdown button:hover {
    background: var(--bg-hover);
  }

  .dropdown .danger {
    color: var(--danger);
  }

  .card-actions {
    display: flex;
    justify-content: flex-end;
  }

  .connect {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-fg);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.8rem;
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .muted,
  .last-used {
    color: var(--fg-muted);
  }

  .last-used {
    flex-shrink: 0;
    font-size: 13px;
  }
</style>
