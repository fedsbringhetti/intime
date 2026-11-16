<script lang="ts">
  import { createTracking, errorText, type Departure } from '../api'
  import { displayName, formatTime } from '../format'
  import { describeDelayLong } from '../lines'
  import { navigate } from '../navigation.svelte'
  import { pathOf } from '../router'
  import {
    DEFAULT_DELAY,
    DEFAULT_REMINDERS,
    DELAY_MAX,
    DELAY_MIN,
    REMINDER_CHOICES,
    buildTrackRequest,
    clampDelay,
    toggleReminder,
  } from '../tracking'
  import LineBadge from './LineBadge.svelte'

  interface Props {
    departure: Departure
    /** Full id of the stop: `gtt:39`. */
    stop: string
    onclose: () => void
  }

  let { departure, stop, onclose }: Props = $props()

  let reminders = $state<number[]>([...DEFAULT_REMINDERS])
  let delay = $state(DEFAULT_DELAY)
  let sending = $state(false)
  let failure = $state<string | null>(null)

  let delayInfo = $derived(departure.realtime ? describeDelayLong(departure.delay_seconds) : null)
  const sourceText = {
    gtt: 'Previsione GTT',
    otp: 'Previsione OpenTripPlanner',
    scheduled: 'Orario programmato',
  }

  async function submit() {
    if (sending) return
    sending = true
    failure = null
    try {
      const tracking = await createTracking(
        buildTrackRequest(stop, departure.trip_id, reminders, delay),
      )
      navigate(pathOf({ name: 'tracking', id: String(tracking.id) }))
    } catch (error) {
      failure = errorText(error)
      sending = false
    }
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="track-h">
  <span class="grabber" aria-hidden="true"></span>
  <h2 id="track-h" class="sr">Segui questa corsa</h2>
  <div class="head">
    <LineBadge name={departure.line} mode={departure.mode} />
    <span class="headsign">{departure.headsign ? displayName(departure.headsign) : ''}</span>
  </div>
  <p class="times">
    <span class="expected">{formatTime(departure.expected)}</span>
    <span class="sub">
      programmata {formatTime(departure.scheduled)}
      {#if delayInfo}· <span class="delay {delayInfo.level}">{delayInfo.label}</span>{/if}
    </span>
  </p>
  <p class="source">{sourceText[departure.source]}</p>

  <section aria-labelledby="rem-h">
    <h3 id="rem-h">Ricordamelo prima della partenza</h3>
    <div class="chips">
      {#each REMINDER_CHOICES as minutes (minutes)}
        <button
          type="button"
          class="chip"
          class:on={reminders.includes(minutes)}
          aria-pressed={reminders.includes(minutes)}
          onclick={() => (reminders = toggleReminder(reminders, minutes))}
        >
          {minutes} min
        </button>
      {/each}
    </div>
  </section>

  <section aria-labelledby="delay-h">
    <h3 id="delay-h">Avvisami se ritarda di almeno</h3>
    <div class="stepper">
      <button
        type="button"
        aria-label="Meno"
        disabled={delay <= DELAY_MIN}
        onclick={() => (delay = clampDelay(delay - 1))}>−</button
      >
      <span class="value" aria-live="polite">{delay} min</span>
      <button
        type="button"
        aria-label="Più"
        disabled={delay >= DELAY_MAX}
        onclick={() => (delay = clampDelay(delay + 1))}>+</button
      >
    </div>
  </section>

  {#if failure}<p class="problem" role="alert">{failure}</p>{/if}
  <button type="button" class="primary" onclick={submit} disabled={sending}>
    Avvisami per questa corsa
  </button>
  <button type="button" class="close" onclick={onclose}>Annulla</button>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: rgba(0, 0, 0, 0.4);
  }

  .sheet {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 31;
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
    margin: 0 auto;
    padding: 8px var(--gutter) calc(16px + env(safe-area-inset-bottom));
    border-radius: var(--radius-card) var(--radius-card) 0 0;
    background: var(--surface);
    color: var(--text);
    max-height: 90vh;
    overflow-y: auto;
  }

  .grabber {
    align-self: center;
    width: 36px;
    height: 5px;
    border-radius: 3px;
    background: var(--separator);
  }

  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .headsign {
    min-width: 0;
    font-size: var(--size-body);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .times {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0;
  }

  .expected {
    font-size: 28px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .sub,
  .source {
    font-size: var(--size-secondary);
    color: var(--text-2);
  }

  .source {
    margin: -8px 0 0;
  }

  .delay {
    font-weight: 600;
  }
  .on-time {
    color: var(--on-time);
  }
  .late {
    color: var(--late);
  }
  .very-late {
    color: var(--very-late);
  }
  .early {
    color: var(--early);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  h3 {
    margin: 0;
    font-size: var(--size-secondary);
    font-weight: 600;
  }

  .chips {
    display: flex;
    gap: 8px;
  }

  .chip {
    flex: 1;
    height: var(--touch);
    border: 0;
    border-radius: 22px;
    background: var(--fill);
    color: var(--text);
    font-size: var(--size-secondary);
    font-weight: 600;
  }

  .chip.on {
    background: var(--accent);
    color: var(--bg);
  }

  .stepper {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .stepper button {
    width: var(--touch);
    height: var(--touch);
    border: 0;
    border-radius: 50%;
    background: var(--fill);
    color: var(--accent);
    font-size: 22px;
    font-weight: 600;
  }

  .stepper button:disabled {
    opacity: 0.4;
  }

  .value {
    font-size: var(--size-body);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .primary {
    width: 100%;
    height: 50px;
    border: 0;
    border-radius: var(--radius-field);
    background: var(--accent);
    color: var(--bg);
    font-size: var(--size-body);
    font-weight: 600;
  }

  .primary:disabled {
    opacity: 0.6;
  }

  .problem {
    margin: 0;
    font-size: var(--size-caption);
    color: var(--danger);
  }

  .close {
    height: var(--touch);
    border: 0;
    background: transparent;
    color: var(--accent);
    font-size: var(--size-body);
  }
</style>
