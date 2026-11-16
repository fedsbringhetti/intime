/**
 * Paths of the web app and the screen each one shows. Pure functions, so they are tested
 * without a browser; `navigation.svelte.ts` connects them to the address bar.
 */

export type Route =
  | { name: 'home' }
  | { name: 'search'; query: string }
  /** `lines`: the line filter, `?linee=4,15`; empty for all lines. */
  | { name: 'stop'; stop: string; lines: string[] }
  | { name: 'near' }
  | { name: 'favorites' }
  | { name: 'notifications' }
  /** The live page of a followed run, `/corsa/12`. */
  | { name: 'tracking'; id: string }
  | { name: 'info' }
  | { name: 'design' }
  | { name: 'not-found' }

/** The five entries of the bottom bar. */
export type Tab = 'home' | 'search' | 'near' | 'favorites' | 'notifications'

export interface RouteOptions {
  /** The design page exists only in development builds. */
  dev: boolean
}

/** Reads a path such as `/fermata/39` (and the query, for `/cerca?q=...`). */
export function matchRoute(pathname: string, search: string, options: RouteOptions): Route {
  // `/cerca/` and `/cerca` are the same screen.
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname
  switch (path) {
    case '/':
      return { name: 'home' }
    case '/cerca':
      return { name: 'search', query: new URLSearchParams(search).get('q') ?? '' }
    case '/vicino':
      return { name: 'near' }
    case '/preferiti':
      return { name: 'favorites' }
    case '/notifiche':
      return { name: 'notifications' }
    case '/info':
      return { name: 'info' }
    case '/prova-grafica':
      return options.dev ? { name: 'design' } : { name: 'not-found' }
  }
  const stop = /^\/fermata\/([^/]+)$/.exec(path)
  if (stop) {
    try {
      return { name: 'stop', stop: decodeURIComponent(stop[1]), lines: readLines(search) }
    } catch {
      // A malformed escape such as `%E0` cannot be a stop.
      return { name: 'not-found' }
    }
  }
  const tracking = /^\/corsa\/(\d+)$/.exec(path)
  if (tracking) return { name: 'tracking', id: tracking[1] }
  return { name: 'not-found' }
}

function readLines(search: string): string[] {
  const value = new URLSearchParams(search).get('linee') ?? ''
  return value
    .split(',')
    .map((line) => line.trim())
    .filter((line) => line !== '')
}

/** The path of a route, for links: the inverse of `matchRoute`. */
export function pathOf(route: Route): string {
  switch (route.name) {
    case 'home':
      return '/'
    case 'search':
      return route.query ? `/cerca?q=${encodeURIComponent(route.query)}` : '/cerca'
    case 'stop': {
      const path = `/fermata/${encodeURIComponent(route.stop)}`
      // Commas stay readable in the address: `?linee=4,15`.
      return route.lines.length
        ? `${path}?linee=${route.lines.map(encodeURIComponent).join(',')}`
        : path
    }
    case 'near':
      return '/vicino'
    case 'favorites':
      return '/preferiti'
    case 'notifications':
      return '/notifiche'
    case 'tracking':
      return `/corsa/${encodeURIComponent(route.id)}`
    case 'info':
      return '/info'
    case 'design':
      return '/prova-grafica'
    case 'not-found':
      return '/'
  }
}

/**
 * The bottom bar entry to highlight. The five main screens are their own entry; a stop or
 * the info screen keeps the entry they were opened from, as in the iPhone's own apps.
 */
export function tabOf(route: Route, previous: Tab): Tab {
  switch (route.name) {
    case 'home':
    case 'search':
    case 'near':
    case 'favorites':
    case 'notifications':
      return route.name
    default:
      return previous
  }
}

/** What `navigation.svelte.ts` needs to know about a click on a link. */
export interface LinkClick {
  button: number
  modifier: boolean
  defaultPrevented: boolean
  /** The link's `target` attribute, `''` when missing. */
  target: string
  download: boolean
  /** The link's absolute URL and the page's origin. */
  url: URL
  origin: string
}

/**
 * Whether a click on a link should change screen inside the app instead of loading a new
 * page: a plain left click on a link to this site, outside `/api/`.
 */
export function isInternalClick(click: LinkClick): boolean {
  return (
    click.button === 0 &&
    !click.modifier &&
    !click.defaultPrevented &&
    (click.target === '' || click.target === '_self') &&
    !click.download &&
    click.url.origin === click.origin &&
    !click.url.pathname.startsWith('/api/')
  )
}
