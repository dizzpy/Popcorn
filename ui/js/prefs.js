// Display preferences, kept on this Mac like the theme. Screens read them
// when they are built, so a change shows the next time a screen opens.

const STORAGE_KEY = 'popcorn.prefs';

const DEFAULTS = {
  /** On Home, show "Title: Subtitle" as a big title over a smaller subtitle. */
  splitTitles: true,
};

function saved() {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY)) ?? {};
  } catch {
    return {};
  }
}

export function pref(name) {
  return saved()[name] ?? DEFAULTS[name];
}

export function setPref(name, value) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...saved(), [name]: value }));
}
