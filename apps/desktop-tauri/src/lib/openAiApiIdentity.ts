/** Mask OpenAI Admin project ids in generic identity labels when privacy is on. */
export function hideOpenAiApiProjectId(
  value: string | null,
  hidePersonalInfo: boolean,
): string | null {
  if (!hidePersonalInfo || !value) return value;
  return value.replace(/^(Admin API|Project):(\s*).+$/i, "$1:$2••••");
}
