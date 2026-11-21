import { invoke } from "./api.js";

// what a replay shows: where things are and what was clicked. All text and every input is masked, pictures, documents
// and embedded pages are left out, and the vendored recorder drops the attributes that could hold personal data.
const RECORDING = {
  record_sessions_percent: 0,
  record_mask_all_text: true,
  record_mask_inputs: true,
  record_block_selector: "img, video, audio, canvas, iframe, embed, object, picture",
  record_canvas: false,
  record_collect_fonts: false,
  record_console: false,
  record_network: false,
  record_heatmap_data: false,
};

let mixpanel = null;

/** The replay being recorded, which the backend names in its events. */
export function replayId() {
  return mixpanel?.get_session_recording_properties?.().$mp_replay_id ?? null;
}

/** Starts or stops recording the session replay, as the beta data collection is on or off. */
export async function syncReplay() {
  const config = await invoke("beta_replay_config").catch(() => null);
  if (!config) {
    mixpanel?.stop_session_recording();
    return;
  }
  if (!mixpanel) {
    const library = (await import("../vendor/mixpanel.module.js")).default;
    library.init(config.token, { api_host: config.api_host, autocapture: false, track_pageview: false, persistence: "localStorage", ...RECORDING });
    library.identify(config.distinct_id);
    mixpanel = library;
  }
  mixpanel.start_session_recording();
}
