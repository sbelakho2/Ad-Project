import { http, HttpResponse, delay } from 'msw';

export const state = {
  lastAuctionBody: null as any,
};

export const handlers = [
  http.post('*/auction', async ({ request }) => {
    try {
      const json = await request.json();
      state.lastAuctionBody = json as any;
    } catch {
      // ignore
    }
    return HttpResponse.json({
      requestId: 'req-123',
      fill: true,
      price: 1.23,
      currency: 'USD',
      creative: { id: 'cr-1', html: '<div>Ad</div>' },
      ttlSeconds: 30,
    });
  }),
];

export const timeoutHandler = http.post('*/auction', async () => {
  await delay(5000);
  return HttpResponse.json({ requestId: 'late', fill: false });
});
