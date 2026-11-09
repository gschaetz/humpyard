/** The admin API contract versions this build was written for (`info.version` of docs/openapi/admin-v1.json). */
export const SUPPORTED_CONTRACT_VERSIONS: readonly string[] = ["1"];

/**
 * What to tell the operator when the gateway's contract is not one this build knows: both versions, named. The data
 * is still shown, because `humpyard ui` can run at a different version from its gateway and a slightly newer or
 * older one is not a dead end. `null` when the versions agree.
 */
export function contractNotice(gateway: string, supported: readonly string[] = SUPPORTED_CONTRACT_VERSIONS): string | null {
  if (supported.includes(gateway)) return null;
  return `This gateway speaks admin API version ${gateway}; this GUI was built for ${supported.join(", ")}. What is shown may be incomplete or wrong.`;
}
