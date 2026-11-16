/** Pure helpers of the "follow a run" feature, tested without a browser. */

import type { TrackRequest } from './api'

/** The reminders offered by the sheet, and the ones on when it opens. */
export const REMINDER_CHOICES = [15, 10, 5, 2] as const
export const DEFAULT_REMINDERS: readonly number[] = [10, 5]
export const DELAY_MIN = 1
export const DELAY_MAX = 15
export const DEFAULT_DELAY = 3

/** Adds or removes a reminder, keeping the list descending. */
export function toggleReminder(chosen: readonly number[], minutes: number): number[] {
  const next = chosen.includes(minutes) ? chosen.filter((m) => m !== minutes) : [...chosen, minutes]
  return next.sort((a, b) => b - a)
}

/** The delay alert stays within 1–15 minutes. */
export function clampDelay(minutes: number): number {
  return Math.min(DELAY_MAX, Math.max(DELAY_MIN, Math.round(minutes)))
}

/** The body of `POST /api/trackings` for the sheet's choices. */
export function buildTrackRequest(
  stop: string,
  tripId: string,
  reminders: readonly number[],
  delayMinutes: number,
): TrackRequest {
  return {
    stop,
    trip_id: tripId,
    alert_before_minutes: [...reminders].sort((a, b) => b - a),
    delay_alert_minutes: clampDelay(delayMinutes),
  }
}

/**
 * "Parte tra N min" for a departure at `expected`, as of `now` (ms). Whole minutes, rounded
 * down as in the departure lists; under a minute is "Sta per partire", after it "Partita".
 */
export function formatCountdown(expected: string, now: number): string {
  const diff = new Date(expected).getTime() - now
  if (diff < -60_000) return 'Partita'
  if (diff < 60_000) return 'Sta per partire'
  return `Parte tra ${Math.floor(diff / 60_000)} min`
}
