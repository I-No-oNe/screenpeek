// Screenpeek substitutes the callback bus name before loading this one-shot script.
const windows = workspace.stackingOrder
    .filter(window => !window.minimized && !window.deleted && !window.specialWindow && window.opacity > 0
        && (window.onAllDesktops || window.desktops.includes(workspace.currentDesktop))
        && (window.activities.length === 0 || window.activities.includes(workspace.currentActivity)))
    .map((window, stack) => {
        const rect = window.clientGeometry;
        return {
            title: window.caption,
            at: [Math.round(rect.x), Math.round(rect.y)],
            size: [Math.round(rect.width), Math.round(rect.height)],
            mapped: true,
            focusHistoryID: window === workspace.activeWindow ? 0 : 1,
            pid: window.pid || null,
            address: window.internalId.toString(),
            stack,
        };
    });
callDBus(SCREENPEEK_CALLBACK, '/org/screenpeek/Windows',
    'org.screenpeek.Windows', 'Reply', JSON.stringify(windows));
