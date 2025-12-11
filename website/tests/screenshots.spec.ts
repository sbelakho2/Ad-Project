import { test } from '@playwright/test';

const pages = [
  { name: 'home', path: '/' },
  { name: 'product', path: '/product' },
  { name: 'pricing', path: '/pricing' },
  { name: 'developers', path: '/developers' },
  { name: 'company', path: '/company' },
  { name: 'security', path: '/security' },
  { name: 'contact', path: '/contact' },
];

test.describe('Website screenshots', () => {
  for (const page of pages) {
    test(`screenshot ${page.name} page`, async ({ page: browserPage }) => {
      await browserPage.goto(page.path, { waitUntil: 'networkidle' });
      
      // Wait for animations to settle
      await browserPage.waitForTimeout(1000);
      
      // Take full page screenshot
      await browserPage.screenshot({
        path: `./screenshots/${page.name}-full.png`,
        fullPage: true,
      });
      
      // Take viewport screenshot
      await browserPage.screenshot({
        path: `./screenshots/${page.name}-viewport.png`,
        fullPage: false,
      });
    });
  }
});
