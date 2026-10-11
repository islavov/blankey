// Light or dark: the system setting until the user picks one with the sidebar switch (remembered on this machine).

const KEY = "blankey.theme";
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

function stored() {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export const currentTheme = () => stored() || (systemDark.matches ? "dark" : "light");

export function applyTheme() {
  document.documentElement.dataset.theme = currentTheme();
}

export function setTheme(theme) {
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // private storage unavailable: the choice lasts until the window reloads
  }
  applyTheme();
}

systemDark.addEventListener("change", applyTheme);
