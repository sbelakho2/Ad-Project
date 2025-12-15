import React from 'react'
import { render } from '@testing-library/react'
import * as jestAxe from 'jest-axe'
import { Select } from '../Select'

const axe = jestAxe.axe
const toHaveNoViolations = (jestAxe as any).toHaveNoViolations

expect.extend(toHaveNoViolations)

describe('Select (ui-v2) a11y', () => {
  it('has no basic accessibility violations', async () => {
    const selectId = 'select-status'
    const { container } = render(
      <div>
        <label htmlFor={selectId}>Status</label>
        <Select id={selectId} defaultValue="active">
          <option value="active">Active</option>
          <option value="paused">Paused</option>
        </Select>
      </div>
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
