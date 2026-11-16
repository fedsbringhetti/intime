<script lang="ts">
  import { onMount } from 'svelte'
  import {
    ApiError,
    deleteTracking,
    errorText,
    getTracking,
    type TrackingDetail,
    type TrackingEvent,
  } from '../lib/api'
  import ErrorCard from '../lib/components/ErrorCard.svelte'
  import Icon from '../lib/components/Icon.svelte'
  import LineBadge from '../lib/components/LineBadge.svelte'
  import SkeletonRows from '../lib/components/SkeletonRows.svelte'
  import { displayName, formatTime } from '../lib/format'
  import { describeDelayLong } from '../lib/lines'
  import { goBack } from '../lib/navigation.svelte'
  import { startPolling } from '../lib/poll'
  import { formatCountdown } from '../lib/tracking'

  interface Props {
    id: string
  }

  let { id }: Props = $props()

  const REFRESH_MS = 15_000
  const SOURCE = {
    gtt: 'Previsione GTT',
    otp: 'Previsione OpenTripPlanner',
    scheduled: 'Orario programmato',
  }

  let data = $state<TrackingDetail | null>(null)
  let failure = $state<string | null>(null)
  let gone = $state(false)
  let stopping = $state(false)
  let stopFailure = $state<string | null>(null)
  let now = $state(Date.now())
  let reload: () => void = () => {}

  onMount(() => {
    const ticker = setInterval(() => (now = Date.now()), 10_000)
    const call = new AbortController()
    reload = () => {
      getTracking(id, call.signal).then(
        (detail) => {
          data = detail
          failure = null
        },
        (error: unknown) => {
          if (call.signal.aborted) return
          if (error instanceof ApiError && error.status === 404) gone = true
          else failure = errorText(error)
        },
      )
    }
    const stop = startPolling(() => {
      if (!gone) reload()
    }, REFRESH_MS)
    return () => {
      clearInterval(ticker)
      call.abort()
      stop()
    }
  })

  async function unfollow() {
    if (stopping) return
    stopping = true
    stopFailure = null
    try {
      await deleteTracking(id)
      goBack('/notifiche')
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) {
        goBack('/notifiche')
        return
      }
      stopFailure = errorText(error)
      stopping = false
    }
  }

  function back(event: MouseEvent) {
    event.preventDefault()
    goBack('/notifiche')
  }

  let delayInfo = $derived(
    data && data.delay_seconds !== null && data.source !== 'scheduled'
      ? describeDelayLong(data.delay_seconds)
      : null,
  )
  let active = $derived(data !== null && ['waiting', 'searching', 'tracking'].includes(data.phase))

  const PHASE_TEXT: Record<string, string> = {
    done: 'La corsa è partita.',
    cancelled: 'La corsa è stata soppressa.',
    not_found: 'La corsa non risulta più nei dati di GTT.',
    stopped: 'Non segui più questa corsa.',
    failed: 'Non riesco più a seguire questa corsa.',
  }

  function eventLabel(event: TrackingEvent): string {
    return event.text
  }
</script>

<header>
  <div class="bar">
    <a class="back" href="/notifiche" onclick={back}>
      <Icon name="back" size={22} stroke={2.4} />Notifiche
    </a>
  </div>
  <h1>Corsa seguita</h1>
</header>

{#if gone}
  <p class="card message">Corsa non più seguita</p>
{:else if data}
  <section class="card summary" aria-label="Corsa">
    <div class="head">
      <LineBadge name={data.line} mode={null} />
      <span class="headsign">{data.headsign ? displayName(data.headsign) : ''}</span>
    </div>
    <p class="stopname">da {displayName(data.stop.name)}</p>

    {#if data.expected}
      {#if active}<p class="countdown">{formatCountdown(data.expected, now)}</p>{/if}
      <p class="times">
        <span class="expected">{formatTime(data.expected)}</span>
        {#if data.scheduled}
          <span class="sub">programmata {formatTime(data.scheduled)}</span>
        {/if}
        {#if delayInfo}<span class="delay {delayInfo.level}">{delayInfo.label}</span>{/if}
      </p>
      {#if data.source}
        <p class="sub">
          {SOURCE[data.source]}{#if data.checked_at}
            · aggiornata alle {formatTime(data.checked_at)}{/if}
        </p>
      {/if}
    {:else}
      <p class="sub">Cerco la corsa…</p>
    {/if}

    {#if !active && PHASE_TEXT[data.phase]}
      <p class="phase">{PHASE_TEXT[data.phase]}</p>
    {/if}
  </section>

  {#if data.vehicle}
    <section class="card vehicle" aria-label="Mezzo">
      <h2>Mezzo assegnato</h2>
      {#if data.vehicle.stops_away !== null}
        <p class="away">
          a {data.vehicle.stops_away}
          {data.vehicle.stops_away === 1 ? 'fermata' : 'fermate'}
        </p>
      {/if}
      <p class="sub">
        Mezzo {data.vehicle.label} · posizione delle {formatTime(data.vehicle.updated_at)}
      </p>
    </section>
  {/if}

  <section class="card alerts" aria-label="Avvisi">
    <h2>Avvisi</h2>
    <ul>
      {#each data.events as event (event.sent_at + event.kind + event.minutes)}
        <li>
          <span class="when">{formatTime(event.sent_at)}</span>
          <span>{eventLabel(event)}</span>
        </li>
      {/each}
      {#if active}
        {#each data.planned_reminders as minutes (minutes)}
          <li class="planned">
            <span class="when">{minutes} min prima</span>
            <span>in programma</span>
          </li>
        {/each}
        <li class="planned">
          <span class="when">Ritardo di {data.delay_alert_minutes} min</span>
          <span>solo se succede</span>
        </li>
      {/if}
    </ul>
  </section>

  {#if failure}<p class="problem" role="alert">{failure}</p>{/if}
  {#if stopFailure}<p class="problem" role="alert">{stopFailure}</p>{/if}
  <button type="button" class="danger" onclick={unfollow} disabled={stopping}>
    Smetti di seguire
  </button>
{:else if failure}
  <ErrorCard message={failure} retry={() => reload()} />
{:else}
  <div class="card"><SkeletonRows count={3} /></div>
{/if}

<style>
  header {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .bar {
    display: flex;
    align-items: center;
    min-height: var(--touch);
    margin: 0 -8px;
  }

  .back {
    display: flex;
    align-items: center;
    gap: 2px;
    height: var(--touch);
    padding: 0 8px;
    color: var(--accent);
  }

  h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.4px;
  }

  h2 {
    margin: 0;
    font-size: var(--size-body);
    font-weight: 700;
  }

  p {
    margin: 0;
  }

  .message {
    padding: 20px var(--gutter);
    text-align: center;
    color: var(--text-2);
  }

  .summary,
  .vehicle,
  .alerts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px var(--gutter);
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

  .stopname,
  .sub {
    font-size: var(--size-secondary);
    color: var(--text-2);
  }

  .countdown {
    font-size: 26px;
    font-weight: 700;
    color: var(--accent);
  }

  .times {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
  }

  .expected {
    font-size: 28px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
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

  .phase {
    font-weight: 600;
  }

  .away {
    font-size: 20px;
    font-weight: 700;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    gap: 10px;
    font-size: var(--size-secondary);
  }

  .when {
    flex-shrink: 0;
    min-width: 92px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  .planned {
    color: var(--text-2);
  }

  .problem {
    font-size: var(--size-caption);
    color: var(--danger);
  }

  .danger {
    width: 100%;
    height: 50px;
    border: 0;
    border-radius: var(--radius-field);
    background: var(--fill);
    color: var(--danger);
    font-size: var(--size-body);
    font-weight: 600;
  }

  .danger:disabled {
    opacity: 0.6;
  }
</style>
