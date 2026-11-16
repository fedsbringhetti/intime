<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { errorText, getTrackings, sendTestNotification, type TrackingSummary } from '../lib/api'
  import ErrorCard from '../lib/components/ErrorCard.svelte'
  import Icon from '../lib/components/Icon.svelte'
  import ScreenHeader from '../lib/components/ScreenHeader.svelte'
  import LineBadge from '../lib/components/LineBadge.svelte'
  import SkeletonRows from '../lib/components/SkeletonRows.svelte'
  import { displayName, formatTime } from '../lib/format'
  import { enablePush, loadPush, type PushState, type PushStatus } from '../lib/push'

  /** `null` while the state is being read. */
  let mode = $state<PushState | null>(null)
  let key = $state<PushStatus['key']>(null)
  let failure = $state<string | null>(null)
  /** "Attiva notifiche" is waiting for the phone's prompt or the server. */
  let enabling = $state(false)
  let enableFailure = $state<string | null>(null)
  let test = $state<'idle' | 'sending' | 'sent'>('idle')
  let testFailure = $state<string | null>(null)
  /** Devices the last test did not reach, when it reached at least one. */
  let testFailed = $state(0)
  let sentTimer: ReturnType<typeof setTimeout> | undefined
  onDestroy(() => clearTimeout(sentTimer))

  async function load() {
    failure = null
    try {
      const status = await loadPush()
      mode = status.state
      key = status.key
    } catch (error) {
      failure = errorText(error)
    }
  }

  /** Followed runs; failing to read them never hides the rest of the screen. */
  let trackings = $state<TrackingSummary[]>([])
  async function loadTrackings() {
    try {
      trackings = await getTrackings()
    } catch {
      trackings = []
    }
  }
  onMount(loadTrackings)

  // Not a button of its own: a screen that opens must never ask for permission, only a tap.
  onMount(load)

  async function enable() {
    if (!key || enabling) return
    enabling = true
    enableFailure = null
    try {
      mode = await enablePush(key)
    } catch (error) {
      enableFailure = errorText(error)
    } finally {
      enabling = false
    }
  }

  async function sendTest() {
    if (test === 'sending') return
    test = 'sending'
    testFailure = null
    testFailed = 0
    clearTimeout(sentTimer)
    try {
      const result = await sendTestNotification()
      if (result.sent > 0) {
        test = 'sent'
        testFailed = result.failed
        sentTimer = setTimeout(() => (test = 'idle'), 4000)
      } else {
        test = 'idle'
        testFailure = 'La prova non è arrivata. Riprova.'
      }
    } catch (error) {
      test = 'idle'
      testFailure = errorText(error)
    }
  }
</script>

<ScreenHeader title="Notifiche" />

{#if failure}
  <ErrorCard message={failure} retry={load} />
{:else if mode === null}
  <div class="card"><SkeletonRows count={2} /></div>
{:else if mode === 'on'}
  <section class="card device" aria-label="Questo dispositivo">
    <span class="status">
      <span class="ok"><Icon name="ok" size={18} stroke={2.2} />Attive</span>
      <span class="sub">su questo telefono</span>
    </span>
    <button type="button" class="chip" onclick={sendTest} disabled={test === 'sending'}>
      {test === 'sent' ? 'Inviata' : 'Invia una prova'}
    </button>
  </section>
  {#if testFailed > 0}
    <p class="problem" role="alert">
      Non arrivata su {testFailed} {testFailed === 1 ? 'dispositivo' : 'dispositivi'}
    </p>
  {/if}
  {#if testFailure}<p class="problem" role="alert">{testFailure}</p>{/if}
{:else}
  {#if mode === 'off'}
    <section class="card enable" aria-labelledby="enable-h">
      <span class="bell"><Icon name="bell" size={30} /></span>
      <h2 id="enable-h">Ricevi gli avvisi su questo telefono</h2>
      <p>
        Riepiloghi a orari fissi e avvisi sulle corse che segui: quando stanno per passare, se
        sono in ritardo o cancellate.
      </p>
      <button type="button" class="primary" onclick={enable} disabled={enabling}>
        Attiva notifiche
      </button>
      <span class="hint">Il telefono ti chiederà il permesso.</span>
      {#if enableFailure}<span class="problem" role="alert">{enableFailure}</span>{/if}
    </section>
  {:else if mode === 'ios-home-screen'}
    <a class="warning" href="/info">
      <Icon name="phone" size={22} />
      <span>
        Su iPhone le notifiche arrivano solo se apri InTime dalla schermata Home.
        <strong>Come si fa</strong>
      </span>
      <Icon name="forward" size={16} stroke={2.6} />
    </a>
  {:else}
    <!-- States without a mockup (decision AQ): the same card, text only. -->
    <section class="card enable" aria-labelledby="enable-h">
      <span class="bell"><Icon name="bell" size={30} /></span>
      {#if mode === 'denied'}
        <h2 id="enable-h">Notifiche bloccate</h2>
        <p>
          Il browser ha bloccato le notifiche di InTime. Per riceverle, consentile dalle
          impostazioni del telefono o del browser, poi torna qui.
        </p>
      {:else if mode === 'server-disabled'}
        <h2 id="enable-h">Notifiche non attive</h2>
        <p>Il server non ha le notifiche attivate: mancano le chiavi di invio.</p>
      {:else}
        <h2 id="enable-h">Notifiche non disponibili</h2>
        <p>Questo browser non supporta le notifiche push. Prova con Chrome o Safari aggiornati.</p>
      {/if}
    </section>
  {/if}
{/if}

{#if trackings.length > 0}
  <section class="followed" aria-labelledby="followed-h">
    <h2 id="followed-h" class="section">Corse seguite</h2>
    <ul class="card">
      {#each trackings as tracking (tracking.id)}
        <li>
          <a class="tracked" href="/corsa/{tracking.id}">
            <LineBadge name={tracking.line} mode={null} />
            <span class="mid">
              <span class="head">{tracking.headsign ? displayName(tracking.headsign) : displayName(tracking.stop.name)}</span>
              <span class="sub">
                {displayName(tracking.stop.name)}{tracking.recurring ? ' · ogni giorno' : ''}
              </span>
            </span>
            <span class="when">
              {tracking.expected ? formatTime(tracking.expected) : 'in ricerca'}
            </span>
            <Icon name="forward" size={16} stroke={2.6} />
          </a>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .section {
    margin: 0 0 8px;
    font-size: var(--size-section);
    font-weight: 700;
  }

  .followed ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .tracked {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr) auto 16px;
    align-items: center;
    gap: 10px;
    min-height: 64px;
    padding: 0 var(--gutter);
    color: var(--text);
  }

  li + li {
    border-top: 1px solid var(--separator);
  }

  .mid {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .head,
  .sub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sub {
    font-size: 14px;
    color: var(--text-2);
  }

  .when {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .device {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px var(--gutter);
  }

  .status {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .ok {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--on-time);
    font-weight: 600;
  }

  .sub {
    font-size: 14px;
    color: var(--text-2);
  }

  .chip {
    flex-shrink: 0;
    height: 40px;
    padding: 0 14px;
    border: 0;
    border-radius: 20px;
    background: var(--fill);
    color: var(--accent);
    font-size: var(--size-secondary);
    font-weight: 600;
  }

  .enable {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 20px var(--gutter) 16px;
    text-align: center;
  }

  .bell {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: var(--fill);
    color: var(--accent);
  }

  h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
  }

  p {
    margin: 0;
    font-size: var(--size-secondary);
    line-height: 1.4;
    color: var(--text-2);
  }

  .primary {
    width: 100%;
    height: 50px;
    margin-top: 4px;
    border: 0;
    border-radius: var(--radius-field);
    background: var(--accent);
    /* The page background is the opposite of the accent in both themes. */
    color: var(--bg);
    font-size: var(--size-body);
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.6;
  }

  .hint {
    font-size: var(--size-caption);
    color: var(--text-2);
  }

  .problem {
    margin: 0;
    font-size: var(--size-caption);
    color: var(--danger);
  }

  .warning {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) 16px;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--radius-card);
    background: var(--warning-bg);
    color: var(--warning);
    font-size: var(--size-secondary);
    line-height: 1.35;
  }
</style>
