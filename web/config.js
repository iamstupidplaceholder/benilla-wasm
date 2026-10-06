// Edit this URL and reload the client at /; no wasm rebuild is needed.
// Empty string uses the page's origin. A reverse-proxy path prefix is supported.
globalThis.BENILLA_CONFIG = {
  gateway: "",
  // Temporary experiment. Set false and reload to restore the CSS/system cursor.
  persistentPointer: true,
  // Browser audio unlocks on a trusted click/key. Set false to open no audio device.
  audio: true,
   // Shared by all MPQ and metadata requests. Before world entry/loading-cover
   // release, recoverable delivery failures retry indefinitely; attempts/total
   // timeout below apply in-world. Missing/invalid files are not retried.
  assetDelivery: {
    attempts: 3,
    attemptTimeoutMs: 10000,
    totalTimeoutMs: 35000,
    retryDelayMs: 250,
  },
};
