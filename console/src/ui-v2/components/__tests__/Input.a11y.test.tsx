import React from 'react'
import { render } from '@testing-library/react'
import * as jestAxe from 'jest-axe'
import { Input } from '../Input'

const axe = jestAxe.axe
const toHaveNoViolations = (jestAxe as any).toHaveNoViolations

expect.extend(toHaveNoViolations)

describe('Input (ui-v2) a11y', () => {
  it('has no basic accessibility violations', async () => {
    const { container } = render(<Input placeholder="Email" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('marks aria-invalid when invalid prop is set', async () => {
    const { container } = render(<Input invalid aria-label="Field" />)
    const el = container.querySelector('input')!
    expect(el.getAttribute('aria-invalid')).toBe('true')
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
