<script lang="ts">
  import {
    ApiError,
    DEPARTURES_MINUTES,
    addFavorite,
    errorText,
    getDepartures,
    getFavorites,
    getStop,
    isUnreachable,
    removeFavorite,
    type Departure,
    type DeparturesResponse,
    type Favorite,
    type StopInfo,
  } from '../lib/api'
  import { alertsForLines, linesWithAlerts } from '../lib/alerts'
  import AlertsBanner from '../lib/components/AlertsBanner.svelte'
  import BusLoader from '../lib/components/BusLoader.svelte'
  import DepartureRow from '../lib/components/DepartureRow.svelte'
  import ErrorCard from '../lib/components/ErrorCard.svelte'
  import Icon from '../lib/components/Icon.svelte'
  import Notice from '../lib/components/Notice.svelte'
  import OfflineBanner from '../lib/components/OfflineBanner.svelte'
  import ScreenHeader from '../lib/components/ScreenHeader.svelte'
  import SkeletonRows from '../lib/components/SkeletonRows.svelte'
  import TrackSheet from '../lib/components/TrackSheet.svelte'
  import { filterDepartures, isChosen, lineChoices, toggleLine } from '../lib/departures'
  import { matchingFavorite, stopIdOf } from '../lib/favorites'
  import { displayName, formatTime } from '../lib/format'
  import { lastData } from '../lib/lastData'
  import { navigate } from '../lib/navigation.svelte'
  import { startPolling } from '../lib/poll'
  import { pathOf } from '../lib/router'
  import { vehicleType } from '../lib/lines'

  interface Props {
    /** The stop as written in the address: `39` or `gtt:39`. */
    stop: string
    /** The line filter from the address (`?linee=4,15`); empty for all lines. */
    lines: string[]
  }

  let { stop, lines }: Props = $props()

  /** How often departures are reloaded while the screen is visible. */
  const REFRESH_MS = 30_000

  let details = $state<StopInfo | null>(null)
  let data = $state<DeparturesResponse | null>(null)
  /** The last load failed: shown as an error, or as a note above data already on screen. */
  let failure = $state<string | null>(null)
  /** The server cannot be reached and old data is shown: when it was received. */
  let staleSince = $state<string | null>(null)
  let notFound = $state(false)
  /** The departure whose follow sheet is open. */
  let tracked = $state<Departure | null>(null)
  let reload: () => void = () => {}

  // The prop is recomputed whenever the address changes, also when only `?linee=` did; a
  // derived value notifies only when the stop itself changes, so filters do not reload.
  const stopId = $derived(stop)

  // Everything here belongs to one stop: when the address changes to another stop, the
  // returned function stops it and the effect starts again for the new one.
  $effect(() => {
    const id = stopId
    details = null
    data = null
    failure = null
    staleSince = null
    notFound = false

    const isNotFound = (error: unknown) =>
      error instanceof ApiError && error.code === 'stop_not_found'

    // Details (description, lines for the filters) are read once; until they arrive, each
    // reload asks again. Without them the screen still works, with the name from the
    // departures. A plain variable, not state: reading state here would make the effect
    // depend on it and start over.
    const detailsCall = new AbortController()
    let detailsState: 'missing' | 'loading' | 'done' = 'missing'
    const loadDetails = () => {
      if (detailsState !== 'missing') return
      detailsState = 'loading'
      getStop(id, detailsCall.signal)
        .then((info) => {
          details = info
          detailsState = 'done'
        })
        .catch((error: unknown) => {
          if (isNotFound(error)) notFound = true
          detailsState = 'missing'
        })
    }

    let call: AbortController | undefined
    const load = () => {
      loadDetails()
      call?.abort()
      const current = new AbortController()
      call = current
      getDepartures(id, current.signal)
        .then((response) => {
          data = response
          failure = null
          staleSince = null
          lastData.saveStop(id, response)
        })
        .catch((error: unknown) => {
          if (current.signal.aborted) return
          if (isNotFound(error)) notFound = true
          if (isUnreachable(error)) {
            // Without network: the data on screen, or else the last saved ones.
            data ??= lastData.loadStop<DeparturesResponse>(id)?.data ?? null
            if (data) {
              staleSince = data.generated_at
              failure = null
              return
            }
          }
          failure = errorText(error)
        })
    }
    reload = load
    const stopPolling = startPolling(load, REFRESH_MS)

    return () => {
      stopPolling()
      detailsCall.abort()
      call?.abort()
    }
  })

  let name = $derived(details?.name ?? data?.stop.name)
  let code = $derived(details?.code ?? data?.stop.code ?? stop)
  let subtitle = $derived(
    details?.description
      ? `Fermata ${code} · ${displayName(details.description)}`
      : `Fermata ${code}`,
  )
  let choices = $derived(lineChoices(details?.lines ?? [], data?.departures ?? []))
  // Offline, departures already gone are hidden: the list shows only what may still pass.
  let shown = $derived(
    data
      ? filterDepartures(data.departures, lines).filter(
          (departure) => !staleSince || Date.parse(departure.expected) > Date.now(),
        )
      : [],
  )

  // GTT's alerts follow the line filter, like the departures.
  let alerts = $derived(alertsForLines(data?.alerts ?? [], lines))
  let alertLines = $derived(linesWithAlerts(alerts))

  // The star: full when a favorite has this stop with exactly the lines on screen.
  let favorites = $state<Favorite[] | null>(null)
  let savingFavorite = $state(false)
  let favoriteFailure = $state<string | null>(null)
  let favorite = $derived(favorites ? matchingFavorite(favorites, stopIdOf(stop), lines) : undefined)

  $effect(() => {
    const call = new AbortController()
    getFavorites(call.signal)
      .then((list) => (favorites = list))
      .catch(() => {
        // Without the list the star stays hidden; the departures matter more.
      })
    return () => call.abort()
  })

  async function toggleFavorite() {
    savingFavorite = true
    favoriteFailure = null
    try {
      if (favorite) await removeFavorite(favorite.id)
      else await addFavorite(stop, lines)
      favorites = await getFavorites()
    } catch (error) {
      favoriteFailure = errorText(error)
    } finally {
      savingFavorite = false
    }
  }

  function choose(line: string | null) {
    const next = line === null ? [] : toggleLine(lines, line)
    // `replace`: changing the filter is not a new screen for the back button.
    navigate(pathOf({ name: 'stop', stop, lines: next }), { replace: true })
  }
</script>

<ScreenHeader
  title={name ? displayName(name) : `Fermata ${stop}`}
  subtitle={notFound ? undefined : subtitle}
  back
>
  {#snippet actions()}
    {#if favorites && !notFound}
      <button
        type="button"
        class="star"
        class:saved={favorite}
        aria-pressed={!!favorite}
        aria-label={favorite ? 'Togli dai preferiti' : 'Aggiungi ai preferiti'}
        disabled={savingFavorite}
        onclick={toggleFavorite}
      >
        <Icon name="star" size={26} stroke={1.8} filled={!!favorite} />
      </button>
    {/if}
  {/snippet}
</ScreenHeader>

{#if favoriteFailure}
  <Notice message="Preferiti non aggiornati. {favoriteFailure}" retry={toggleFavorite} />
{/if}

{#if notFound}
  <div class="card message">
    <p>La fermata {stop} non esiste, o non è più servita.</p>
    <a href="/cerca">Cerca un'altra fermata</a>
  </div>
{:else}
  {#if choices.length > 1}
    <div class="chips" role="group" aria-label="Filtra per linea">
      <button type="button" class="chip all" aria-pressed={lines.length === 0} onclick={() => choose(null)}>
        Tutte
      </button>
      {#each choices as line (line.name)}
        <button
          type="button"
          class="chip"
          aria-pressed={isChosen(lines, line.name)}
          onclick={() => choose(line.name)}
        >
          <span class="dot {vehicleType(line.mode)}"></span>{line.name}
        </button>
      {/each}
    </div>
  {/if}

  {#if alerts.length > 0}
    <!-- Keyed by stop: another stop starts with the banner closed. -->
    {#key stopId}
      <AlertsBanner {alerts} />
    {/key}
  {/if}

  {#if staleSince}
    <OfflineBanner since={staleSince} />
  {:else if data?.stale}
    <Notice message="Il servizio degli orari non risponde: dati delle {formatTime(data.fetched_at)}." />
  {/if}

  {#if data}
    {#if failure}
      <Notice
        message="Non riesco ad aggiornare: dati delle {formatTime(data.generated_at)}."
        retry={() => reload()}
      />
    {/if}
    {#if shown.length === 0}
      <p class="card message">
        {lines.length
          ? `Nessuna partenza delle linee scelte nei prossimi ${DEPARTURES_MINUTES} minuti.`
          : `Nessuna partenza nei prossimi ${DEPARTURES_MINUTES} minuti.`}
      </p>
    {:else}
      <ol class="card">
        {#each shown as departure (departure.trip_id + departure.scheduled)}
          <DepartureRow
            line={departure.line}
            mode={departure.mode}
            headsign={departure.headsign ? displayName(departure.headsign) : ''}
            time={formatTime(departure.canceled ? departure.scheduled : departure.expected)}
            minutes={departure.minutes}
            delay={departure.delay_seconds}
            realtime={departure.realtime}
            canceled={departure.canceled}
            stale={staleSince !== null}
            alert={alertLines.has(departure.line.toLowerCase())}
            onselect={staleSince ? undefined : () => (tracked = departure)}
          />
        {/each}
      </ol>
    {/if}
    {#if !staleSince}
      <p class="updated">Aggiornato alle {formatTime(data.generated_at)} · si aggiorna da solo</p>
    {/if}
  {:else if failure}
    <ErrorCard message={failure} retry={() => reload()} />
  {:else}
    <div class="card">
      <BusLoader label="Carico le partenze…" />
      <SkeletonRows count={4} />
    </div>
  {/if}
{/if}

{#if tracked && data}
  <TrackSheet departure={tracked} stop={data.stop.id} onclose={() => (tracked = null)} />
{/if}

<style>
  .star {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--touch);
    height: var(--touch);
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
  }

  .star.saved {
    color: var(--favorite);
  }

  .chips {
    display: flex;
    gap: 8px;
    /* One row that scrolls sideways, edge to edge, when the lines do not fit. */
    overflow-x: auto;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter);
    scrollbar-width: none;
  }

  .chip {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 36px;
    padding: 0 12px;
    border-radius: 18px;
    border: 1px solid var(--separator);
    background: var(--surface);
    font-size: var(--size-secondary);
    font-weight: 600;
  }

  .chip.all {
    padding: 0 14px;
  }

  .chip[aria-pressed='true'] {
    border-color: var(--text);
    background: var(--text);
    color: var(--bg);
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 3px;
  }
  .dot.bus {
    background: var(--bus);
  }
  .dot.tram {
    background: var(--tram);
  }
  .dot.metro {
    background: var(--metro);
  }
  .dot.rail {
    background: var(--rail);
  }
  .dot.other {
    background: var(--other);
  }

  .message {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px var(--gutter);
    font-size: var(--size-secondary);
    color: var(--text-2);
  }

  .message p {
    margin: 0;
  }

  .message a {
    color: var(--accent);
    font-weight: 600;
  }

  .updated {
    margin: 0;
    text-align: center;
    font-size: var(--size-caption);
    color: var(--text-2);
  }
</style>
