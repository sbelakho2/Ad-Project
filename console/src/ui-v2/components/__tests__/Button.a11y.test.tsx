import React from 'react'
import { render } from '@testing-library/react'
import * as jestAxe from 'jest-axe'
import { Button } from '../Button'

const axe = jestAxe.axe
const toHaveNoViolations = (jestAxe as any).toHaveNoViolations

expect.extend(toHaveNoViolations)

describe('Button (ui-v2) a11y', () => {
  it('has no basic accessibility violations', async () => {
    const { container } = render(<Button>Click me</Button>)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('exposes proper aria-busy when loading', async () => {
    const { container } = render(<Button isLoading>Submitting</Button>)
    const btn = container.querySelector('button')
    expect(btn?.getAttribute('aria-busy')).toBe('true')
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
