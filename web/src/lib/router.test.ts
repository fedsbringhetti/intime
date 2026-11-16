import { describe, expect, it } from 'vitest'
import { isInternalClick, matchRoute, pathOf, tabOf, type LinkClick, type Route } from './router'

const prod = { dev: false }
const dev = { dev: true }

describe('tracking route', () => {
  it('reads and writes /corsa/{id}', () => {
    expect(matchRoute('/corsa/12', '', prod)).toEqual({ name: 'tracking', id: '12' })
    expect(matchRoute('/corsa/12/', '', prod)).toEqual({ name: 'tracking', id: '12' })
    expect(matchRoute('/corsa/abc', '', prod)).toEqual({ name: 'not-found' })
    expect(matchRoute('/corsa', '', prod)).toEqual({ name: 'not-found' })
    expect(pathOf({ name: 'tracking', id: '12' })).toBe('/corsa/12')
  })

  it('keeps the tab it was opened from', () => {
    expect(tabOf({ name: 'tracking', id: '1' }, 'notifications')).toBe('notifications')
  })
})

describe('matchRoute', () => {
  it('reads the main screens', () => {
    expect(matchRoute('/', '', prod)).toEqual({ name: 'home' })
    expect(matchRoute('/vicino', '', prod)).toEqual({ name: 'near' })
    expect(matchRoute('/preferiti', '', prod)).toEqual({ name: 'favorites' })
    expect(matchRoute('/notifiche', '', prod)).toEqual({ name: 'notifications' })
    expect(matchRoute('/notifiche/', '', prod)).toEqual({ name: 'notifications' })
    expect(matchRoute('/info', '', prod)).toEqual({ name: 'info' })
  })

  it('reads the search query', () => {
    expect(matchRoute('/cerca', '?q=porta%20nuova', prod)).toEqual({
      name: 'search',
      query: 'porta nuova',
    })
    expect(matchRoute('/cerca', '', prod)).toEqual({ name: 'search', query: '' })
  })

  it('reads a stop by code or by full id', () => {
    expect(matchRoute('/fermata/39', '', prod)).toEqual({ name: 'stop', stop: '39', lines: [] })
    expect(matchRoute('/fermata/gtt%3A39', '', prod)).toEqual({
      name: 'stop',
      stop: 'gtt:39',
      lines: [],
    })
  })

  it('reads the line filter of a stop', () => {
    expect(matchRoute('/fermata/39', '?linee=4,15', prod)).toEqual({
      name: 'stop',
      stop: '39',
      lines: ['4', '15'],
    })
    expect(matchRoute('/fermata/39', '?linee=9B,,%20', prod)).toMatchObject({ lines: ['9B'] })
  })

  it('ignores a trailing slash', () => {
    expect(matchRoute('/cerca/', '', prod)).toEqual({ name: 'search', query: '' })
    expect(matchRoute('/fermata/39/', '', prod)).toEqual({ name: 'stop', stop: '39', lines: [] })
  })

  it('shows the design page only in development', () => {
    expect(matchRoute('/prova-grafica', '', dev)).toEqual({ name: 'design' })
    expect(matchRoute('/prova-grafica', '', prod)).toEqual({ name: 'not-found' })
  })

  it('does not know other paths', () => {
    for (const path of ['/fermata', '/fermata/39/linee', '/fermata/%E0', '/stop/39', '/index.html']) {
      expect(matchRoute(path, '', prod), path).toEqual({ name: 'not-found' })
    }
  })
})

describe('pathOf', () => {
  it('is the inverse of matchRoute', () => {
    const routes: Route[] = [
      { name: 'home' },
      { name: 'search', query: '' },
      { name: 'search', query: 'c.so vittorio & co' },
      { name: 'stop', stop: '39', lines: [] },
      { name: 'stop', stop: 'gtt:39', lines: [] },
      { name: 'stop', stop: '39', lines: ['4', '9B'] },
      { name: 'near' },
      { name: 'favorites' },
      { name: 'notifications' },
      { name: 'info' },
      { name: 'design' },
    ]
    for (const route of routes) {
      const url = new URL(pathOf(route), 'http://localhost')
      expect(matchRoute(url.pathname, url.search, dev)).toEqual(route)
    }
  })
})

describe('tabOf', () => {
  it('highlights the main screens themselves', () => {
    expect(tabOf({ name: 'near' }, 'home')).toBe('near')
    expect(tabOf({ name: 'search', query: 'x' }, 'favorites')).toBe('search')
    expect(tabOf({ name: 'notifications' }, 'home')).toBe('notifications')
  })

  it('keeps the previous entry on a stop or the info screen', () => {
    expect(tabOf({ name: 'stop', stop: '39', lines: [] }, 'search')).toBe('search')
    expect(tabOf({ name: 'info' }, 'home')).toBe('home')
  })
})

describe('isInternalClick', () => {
  const click = (changes: Partial<LinkClick> = {}): LinkClick => ({
    button: 0,
    modifier: false,
    defaultPrevented: false,
    target: '',
    download: false,
    url: new URL('http://localhost:5173/fermata/39'),
    origin: 'http://localhost:5173',
    ...changes,
  })

  it('handles a plain click on a link of the app', () => {
    expect(isInternalClick(click())).toBe(true)
  })

  it('leaves the browser alone otherwise', () => {
    expect(isInternalClick(click({ button: 1 }))).toBe(false)
    expect(isInternalClick(click({ modifier: true }))).toBe(false)
    expect(isInternalClick(click({ defaultPrevented: true }))).toBe(false)
    expect(isInternalClick(click({ target: '_blank' }))).toBe(false)
    expect(isInternalClick(click({ download: true }))).toBe(false)
    expect(isInternalClick(click({ url: new URL('https://www.gtt.to.it/') }))).toBe(false)
    expect(isInternalClick(click({ url: new URL('http://localhost:5173/api/status') }))).toBe(
      false,
    )
  })
})
