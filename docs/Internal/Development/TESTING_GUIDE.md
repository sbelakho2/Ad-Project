# Testing Guide - Rival ApexMediation Console

## Quick Start

### 1. Start the Development Server

```bash
./start-dev.sh
```

Or manually:
```bash
cd console
npm run dev
```

The console will be available at: **http://localhost:3000**

### 2. Login

Navigate to http://localhost:3000 (will redirect to login)

**Demo Credentials:**
- Email: `demo@rival.com`
- Password: `demo` (or any password)

## What to Test

### ✅ Dashboard (Home Page)
**URL:** http://localhost:3000/dashboard

**Features to verify:**
- [ ] Revenue metrics cards display (Total Revenue, Impressions, eCPM, Fill Rate)
- [ ] Percentage change indicators (green for positive, red for negative)
- [ ] Date range selector (Last 7/30/90 days)
- [ ] Revenue trend chart with multiple data points
- [ ] Quick action buttons (Create Placement, Settings, Analytics, View Payouts)

**Expected Data:**
- Revenue: $45K - $85K
- Impressions: 2M - 5M
- eCPM: $3.50 - $7.50
- Fill Rate: 82% - 94%

---

### ✅ Placements Page
**URL:** http://localhost:3000/placements

**Features to verify:**
- [ ] List of 3 mock placements
- [ ] Status badges (active/paused)
- [ ] Revenue, impressions, eCPM, fill rate for each
- [ ] Filter by type (banner, interstitial, rewarded)
- [ ] Search functionality
- [ ] "Create Placement" button

**Mock Placements:**
1. Main Menu Banner (Banner)
2. Gameplay Interstitial (Interstitial)
3. Reward Video - Extra Lives (Rewarded)

---

### ✅ Adapters Page
**URL:** http://localhost:3000/adapters

**Features to verify:**
- [ ] Grid of 4 network adapters
- [ ] Revenue and performance metrics for each
- [ ] Status indicators (active/paused)
- [ ] Network logos/icons
- [ ] "Add Adapter" button

**Mock Adapters:**
1. Google AdMob
2. AppLovin MAX
3. Meta Audience Network
4. IronSource

---

### ✅ Analytics Page
**URL:** http://localhost:3000/analytics

**Features to verify:**
- [ ] Date range picker
- [ ] Revenue trend line chart (30 days)
- [ ] eCPM and Fill Rate dual-axis chart
- [ ] Impressions vs Clicks bar chart
- [ ] Summary metrics
- [ ] Export functionality

**Charts to check:**
- Revenue should show daily variation
- eCPM typically ranges $2-$8
- Fill rate typically 75%-95%
- Click data should correlate with impressions

---

### ✅ Fraud Detection Page
**URL:** http://localhost:3000/fraud

**Features to verify:**
- [ ] Time window selector (1h, 24h, 7d, 30d)
- [ ] Fraud rate metric card
- [ ] Blocked revenue amount
- [ ] Detection method breakdown (GIVT, SIVT, ML, Anomaly)
- [ ] Alert list with severity levels
- [ ] Alert status indicators

**Mock Data:**
- Fraud rate: 0.5% - 2.5%
- Multiple alerts with different severity levels
- Detection methods showing distribution

---

### ✅ Payouts Page
**URL:** http://localhost:3000/payouts

**Features to verify:**
- [ ] Upcoming payout card with countdown
- [ ] Payment history table
- [ ] Status badges (pending, processing, completed)
- [ ] Payment method indicators (Mollie, PayPal, Wire)
- [ ] Pagination controls
- [ ] Export history button

**Mock Payouts:**
- Upcoming: $5K-$9K (5 days away)
- History: 3 recent payouts
- Various statuses and methods

---

### ✅ Settings Pages
**URL:** http://localhost:3000/settings

**Sub-pages to test:**

#### 1. Fraud Settings (`/settings/fraud`)
- [ ] Fraud detection toggle
- [ ] Threshold sliders
- [ ] Detection method checkboxes
- [ ] Alert configuration
- [ ] Save button

#### 2. Payout Settings (`/settings/payouts`)
- [ ] Payment method selection
- [ ] Schedule preferences
- [ ] Minimum payout amount
- [ ] Payment details form
- [ ] Save button

#### 3. Team Management (`/settings/team`)
- [ ] Team member list
- [ ] Role assignments
- [ ] Invite new member button
- [ ] Pending invitations
- [ ] Remove member action

#### 4. Notifications (`/settings/notifications`)
- [ ] Email notification toggles
- [ ] Push notification settings
- [ ] Alert preferences
- [ ] Frequency settings
- [ ] Save preferences

#### 5. Compliance (`/settings/compliance`)
- [ ] Privacy policy agreement
- [ ] GDPR compliance toggle
- [ ] COPPA settings
- [ ] Data retention policies
- [ ] Export data button

---

## Navigation Testing

### ✅ Sidebar Navigation
- [ ] Logo/brand at top
- [ ] Active route highlighting
- [ ] All menu items clickable:
  - Dashboard
  - Placements
  - Adapters
  - Analytics
  - Fraud Detection
  - Payouts
  - Settings
- [ ] User profile section at bottom
- [ ] Sign out button

### ✅ Mobile Responsiveness
- [ ] Hamburger menu appears on mobile (<768px)
- [ ] Sidebar toggles on/off
- [ ] Content adapts to smaller screens
- [ ] Cards stack vertically
- [ ] Tables scroll horizontally

---

## Performance Testing

### Load Times
- [ ] Initial page load < 2 seconds
- [ ] Mock API responses 100-500ms
- [ ] Charts render smoothly
- [ ] No layout shifts during loading

### Loading States
- [ ] Skeleton loaders on dashboard
- [ ] Spinner/loading indicators
- [ ] Disabled states during actions
- [ ] Error boundaries catch errors

---

## Visual Testing

### Design System (Aurora Slate)
- [ ] Consistent color scheme (primary blues, accent purples)
- [ ] Typography hierarchy clear
- [ ] Proper spacing and padding
- [ ] Hover states on interactive elements
- [ ] Focus states for accessibility

### Components
- [ ] Buttons have correct styles
- [ ] Cards have shadows and borders
- [ ] Forms have proper validation styling
- [ ] Badges use semantic colors
- [ ] Icons render correctly (Lucide React)

---

## Data Flow Testing

### Mock API Integration
- [ ] Dashboard fetches revenue summary
- [ ] Placements page fetches placement list
- [ ] Adapters page fetches adapter list
- [ ] Fraud page fetches stats and alerts
- [ ] Payouts page fetches history and upcoming
- [ ] All requests use `/api/mock?endpoint=...`

### React Query Caching
- [ ] Data persists on navigation
- [ ] Refresh button refetches data
- [ ] Loading states only on initial fetch
- [ ] Error handling shows user-friendly messages

---

## Error Scenarios

### Test These Cases
- [ ] Navigate without login → redirects to /login
- [ ] Invalid mock endpoint → shows error
- [ ] Network timeout simulation
- [ ] Empty data states
- [ ] Error boundaries catch React errors

---

## Browser Compatibility

Test in:
- [ ] Chrome (latest)
- [ ] Safari (latest)
- [ ] Firefox (latest)
- [ ] Edge (latest)

---

## Accessibility

- [ ] Keyboard navigation works
- [ ] Tab order is logical
- [ ] Focus indicators visible
- [ ] ARIA labels on icons
- [ ] Semantic HTML structure
- [ ] Color contrast meets WCAG AA

---

## Known Limitations (Mock Mode)

### What Works:
✅ All read operations (GET)
✅ Realistic random data
✅ Loading states and delays
✅ Navigation and routing
✅ Authentication (demo mode)

### What Doesn't Work:
❌ Create/Update/Delete operations (no persistence)
❌ File uploads
❌ Real-time updates
❌ WebSocket connections
❌ Payment processing

---

## Troubleshooting

### Server Won't Start
```bash
# Clear cache and reinstall
cd console
rm -rf node_modules .next
npm install
npm run dev
```

### Mock API Not Working
Check `.env.local`:
```
NEXT_PUBLIC_USE_MOCK_API=true
```

### TypeScript Errors
```bash
cd console
npm run type-check
```

### Port 3000 Already in Use
```bash
# Kill process on port 3000
lsof -ti:3000 | xargs kill -9

# Or use different port
PORT=3001 npm run dev
```

---

## Testing Checklist

### Before Marking Complete:
- [ ] All pages load without errors
- [ ] Mock data displays correctly
- [ ] Charts render with data
- [ ] Navigation works smoothly
- [ ] Mobile view is responsive
- [ ] No console errors
- [ ] TypeScript compiles cleanly
- [ ] All links are functional

### Performance Benchmarks:
- [ ] Lighthouse score > 90
- [ ] First Contentful Paint < 1.5s
- [ ] Time to Interactive < 3s
- [ ] No memory leaks on navigation

---

## Next Steps After Testing

1. **Production Backend**: Replace mock API with real backend services
2. **Authentication**: Implement OAuth2/JWT with real API
3. **Real-time Updates**: Add WebSocket connections for live data
4. **Advanced Analytics**: Integrate more sophisticated charting
5. **E2E Tests**: Add Cypress/Playwright tests
6. **CI/CD**: Deploy to staging environment

---

## Support

If you encounter issues:
1. Check browser console for errors
2. Verify `.env.local` configuration
3. Ensure all dependencies installed (`npm install`)
4. Check terminal for server errors
5. Review this testing guide for expected behavior

**Happy Testing! 🚀**
