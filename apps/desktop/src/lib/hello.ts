/**
 * Confirmation between the hold and the undo window.
 * A Windows build calls Hello here. This module is the seam.
 */
export interface WindowsHello {
  confirm(approvalId: string): Promise<boolean>;
}

/**
 * Dev stand-in. No OS prompt is available here.
 * The card draws "Confirm with Windows Hello" and resolves this on Confirm.
 * Cancel never calls it. A false result is the same as cancel: back to waiting.
 */
export const devWindowsHello: WindowsHello = {
  confirm(approvalId: string): Promise<boolean> {
    if (approvalId.length === 0) {
      return Promise.resolve(false);
    }
    return Promise.resolve(true);
  },
};
