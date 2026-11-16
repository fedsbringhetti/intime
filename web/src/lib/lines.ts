/** Vehicle types as the app colors them; each has a `--<type>` color in `app.css`. */
export type VehicleType = 'bus' | 'tram' | 'metro' | 'rail' | 'other'

/**
 * The color group of a line from the upstream `mode` (`BUS`, `TRAM`, `SUBWAY`, `RAIL`,
 * `FUNICULAR`, ...; sometimes missing).
 */
export function vehicleType(mode: string | null | undefined): VehicleType {
  switch (mode?.toUpperCase()) {
    case 'BUS':
    case 'TROLLEYBUS':
    case 'COACH':
      return 'bus'
    case 'TRAM':
      return 'tram'
    case 'SUBWAY':
    case 'METRO':
      return 'metro'
    case 'RAIL':
    case 'MONORAIL':
      return 'rail'
    default:
      return 'other'
  }
}

export type DelayLevel = 'on-time' | 'late' | 'very-late' | 'early'

/** Minutes from which a delay is shown in red instead of amber. */
const VERY_LATE_MINUTES = 5

/**
 * The label and color of a delay in seconds: "in orario", "+2", "−1". Rounded to the
 * nearest minute, so 20 seconds late is still on time.
 */
export function describeDelayLong(seconds: number): { label: string; level: DelayLevel } {
  const d = describeDelay(seconds)
  if (d.level === 'on-time') return d
  return { ...d, label: `${d.label} min ${d.level === 'early' ? 'in anticipo' : 'in ritardo'}` }
}

export function describeDelay(seconds: number): { label: string; level: DelayLevel } {
  const minutes = Math.round(seconds / 60)
  if (minutes === 0) return { label: 'in orario', level: 'on-time' }
  // U+2212 is the minus sign: as wide as "+", unlike the hyphen.
  if (minutes < 0) return { label: `−${-minutes}`, level: 'early' }
  return { label: `+${minutes}`, level: minutes >= VERY_LATE_MINUTES ? 'very-late' : 'late' }
}
