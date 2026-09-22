// Screenpeek substitutes the callback bus name before loading this one-shot script.
const windows = workspace.windowList()
    .filter(window => !window.minimized && !window.deleted && !window.specialWindow
        && (window.onAllDesktops || window.desktops.includes(workspace.currentDesktop))
        && (window.activities.length === 0 || window.activities.includes(workspace.currentActivity)))
    .map(window => {
        const rect = window.clientGeometry;
        return {
            title: window.caption,
            at: [Math.round(rect.x), Math.round(rect.y)],
            size: [Math.round(rect.width), Math.round(rect.height)],
            mapped: true,
            focusHistoryID: window === workspace.activeWindow ? 0 : 1,
            pid: window.pid || null,
        };
    });
callDBus(SCREENPEEK_CALLBACK, '/org/screenpeek/Windows',
    'org.screenpeek.Windows', 'Reply', JSON.stringify(windows));
