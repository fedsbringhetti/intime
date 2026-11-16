import { describe, expect, it } from 'vitest'
import { describeDelay, describeDelayLong, vehicleType } from './lines'

describe('vehicleType', () => {
  it('groups the upstream modes', () => {
    expect(vehicleType('BUS')).toBe('bus')
    expect(vehicleType('TRAM')).toBe('tram')
    expect(vehicleType('SUBWAY')).toBe('metro')
    expect(vehicleType('RAIL')).toBe('rail')
  })

  it('ignores case and falls back to other', () => {
    expect(vehicleType('tram')).toBe('tram')
    expect(vehicleType('FUNICULAR')).toBe('other')
    expect(vehicleType(null)).toBe('other')
    expect(vehicleType(undefined)).toBe('other')
  })
})

describe('describeDelay', () => {
  it('rounds to the nearest minute', () => {
    expect(describeDelay(0)).toEqual({ label: 'in orario', level: 'on-time' })
    expect(describeDelay(29)).toEqual({ label: 'in orario', level: 'on-time' })
    expect(describeDelay(-29)).toEqual({ label: 'in orario', level: 'on-time' })
    expect(describeDelay(90)).toEqual({ label: '+2', level: 'late' })
  })

  it('turns red from 5 minutes', () => {
    expect(describeDelay(4 * 60)).toEqual({ label: '+4', level: 'late' })
    expect(describeDelay(5 * 60)).toEqual({ label: '+5', level: 'very-late' })
    expect(describeDelay(20 * 60)).toEqual({ label: '+20', level: 'very-late' })
  })

  it('uses the minus sign when early', () => {
    expect(describeDelay(-60)).toEqual({ label: '−1', level: 'early' })
    expect(describeDelay(-170)).toEqual({ label: '−3', level: 'early' })
  })
})

describe('describeDelayLong', () => {
  it('adds the words next to the minutes', () => {
    expect(describeDelayLong(0).label).toBe('in orario')
    expect(describeDelayLong(-180).label).toBe('−3 min in anticipo')
    expect(describeDelayLong(540).label).toBe('+9 min in ritardo')
  })
})
