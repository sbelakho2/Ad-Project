import { test, expect } from '@playwright/test';

test.describe('Horizontal Overflow Check', () => {
  test.use({ viewport: { width: 375, height: 667 } });

  test('Home page has no horizontal overflow on mobile', async ({ page }) => {
    await page.goto('https://apexmediation.ee/');
    await page.waitForLoadState('networkidle');
    
    // Get viewport and document widths
    const result = await page.evaluate(() => {
      const viewportWidth = window.innerWidth;
      const documentWidth = document.documentElement.scrollWidth;
      const bodyWidth = document.body.scrollWidth;
      
      // Find elements causing overflow
      const overflowingElements: string[] = [];
      document.querySelectorAll('*').forEach(el => {
        const rect = el.getBoundingClientRect();
        if (rect.right > viewportWidth + 5) {
          overflowingElements.push(`${el.tagName}.${el.className.split(' ').slice(0,3).join('.')} (right: ${Math.round(rect.right)}px)`);
        }
      });
      
      return {
        viewportWidth,
        documentWidth,
        bodyWidth,
        hasOverflow: documentWidth > viewportWidth,
        overflowAmount: documentWidth - viewportWidth,
        overflowingElements: overflowingElements.slice(0, 10) // First 10
      };
    });
    
    console.log('Overflow analysis:', JSON.stringify(result, null, 2));
    
    // Assert no horizontal overflow
    expect(result.hasOverflow, `Page has ${result.overflowAmount}px horizontal overflow. Overflowing elements: ${result.overflowingElements.join(', ')}`).toBe(false);
  });
});
