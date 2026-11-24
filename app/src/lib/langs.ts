/** Subtitle languages offered in Settings and by a Watch together host: [code, name]. */
export const SUB_LANGS: [string, string][] = [
  ["en", "English"],
  ["off", "Off"],
  ["es", "Spanish"],
  ["pt", "Portuguese"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["ar", "Arabic"],
  ["id", "Indonesian"],
  ["ru", "Russian"],
  ["ja", "Japanese"],
];

export const subLangName = (code: string | null | undefined) => SUB_LANGS.find(([c]) => c === code)?.[1] ?? code ?? "";
