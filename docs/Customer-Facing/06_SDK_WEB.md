# Web SDK Integration

## Quick Start

### 1. Add Script Tag

Add the Apex Mediation script to the `<head>` of your HTML page:

```html
<script src="https://cdn.apexmediation.ee/sdk/web/v1/apex.min.js" async></script>
```

### 2. Initialize SDK

Initialize the SDK with your configuration. Note that the `endpoint` is required.

```javascript
window.apex = window.apex || [];
function apex() { window.apex.push(arguments); }

apex('init', {
  appId: 'YOUR_APP_ID',
  endpoint: 'https://api.apexmediation.ee', // Required: Auction endpoint
  publisherId: 'YOUR_PUB_ID', // Optional but recommended
  debug: true, // Enable debug logging
  timeoutMs: 2000 // Auction timeout in milliseconds
});
```

### 3. Define Ad Slots

Place a container `<div>` where you want the ad to appear, and call `defineSlot`.

```html
<!-- Ad Container -->
<div id="ad-slot-1" style="width: 300px; height: 250px;"></div>

<script>
  apex('defineSlot', {
    placementId: 'PLACEMENT_ID',
    containerId: 'ad-slot-1',
    size: [300, 250]
  });
</script>
```

### 4. Display Ads

Once slots are defined, request ads:

```javascript
apex('display', 'ad-slot-1');
```

## Key Concepts

*   **Endpoint**: The Web SDK requires an explicit auction endpoint URL. This allows for white-labeling and enterprise proxy configurations.
*   **Container**: The DOM element where the ad will be rendered. Must have explicit dimensions or the ad may collapse.
*   **Async Loading**: The SDK loads asynchronously to avoid blocking page render. The `window.apex` queue ensures commands are executed once the SDK is ready.

## Single Page Applications (SPA)

For React, Vue, or Angular apps, you can install the NPM package:

```bash
npm install @apexmediation/web-sdk
```

**React Example:**

```jsx
import { init, ApexAd } from '@apexmediation/web-sdk/react';

// Initialize once (e.g., in App.js)
init({
  appId: 'YOUR_APP_ID',
  endpoint: 'https://api.apexmediation.ee'
});

function App() {
  return (
    <div>
      <h1>My App</h1>
      <ApexAd 
        placementId="PLACEMENT_ID" 
        width={300} 
        height={250} 
      />
    </div>
  );
}
```

## Integration Checklist

1.  [ ] Added script tag to `<head>`.
2.  [ ] Initialized with correct `appId` and `endpoint`.
3.  [ ] Created container `<div>`s with correct IDs.
4.  [ ] Verified ads.txt is updated on your domain.
5.  [ ] Verified ads render in supported browsers.

## Debugging

Enable debug mode in the init options:

```javascript
apex('init', {
  // ...
  debug: true
});
```

Check the browser's Developer Tools > Console for logs.
