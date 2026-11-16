import { describe, expect, it } from 'vitest'
import {
  buildTrackRequest,
  clampDelay,
  DEFAULT_DELAY,
  DEFAULT_REMINDERS,
  formatCountdown,
  toggleReminder,
} from './tracking'

describe('sheet payload', () => {
  it('builds the default request', () => {
    expect(buildTrackRequest('gtt:39', 'abc', DEFAULT_REMINDERS, DEFAULT_DELAY)).toEqual({
      stop: 'gtt:39',
      trip_id: 'abc',
      alert_before_minutes: [10, 5],
      delay_alert_minutes: 3,
    })
  })

  it('keeps reminders descending and the delay within 1-15', () => {
    let chosen = toggleReminder([...DEFAULT_REMINDERS], 15)
    chosen = toggleReminder(chosen, 2)
    chosen = toggleReminder(chosen, 10)
    expect(chosen).toEqual([15, 5, 2])
    expect(clampDelay(0)).toBe(1)
    expect(clampDelay(99)).toBe(15)
    expect(buildTrackRequest('s', 't', [], 20).delay_alert_minutes).toBe(15)
  })
})

describe('formatCountdown', () => {
  const now = Date.parse('2026-11-16T10:00:00Z')
  it('counts whole minutes', () => {
    expect(formatCountdown('2026-11-16T10:07:30Z', now)).toBe('Parte tra 7 min')
    expect(formatCountdown('2026-11-16T10:01:00Z', now)).toBe('Parte tra 1 min')
  })
  it('handles the last minute and departed runs', () => {
    expect(formatCountdown('2026-11-16T10:00:30Z', now)).toBe('Sta per partire')
    expect(formatCountdown('2026-11-16T09:58:00Z', now)).toBe('Partita')
  })
})
