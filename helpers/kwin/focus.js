// Screenpeek substitutes the callback bus name and target id before loading this one-shot script.
const target = workspace.windowList().find(window => window.internalId.toString() === SCREENPEEK_TARGET);
if (target) workspace.activeWindow = target;
callDBus(SCREENPEEK_CALLBACK, '/org/screenpeek/Windows',
    'org.screenpeek.Windows', 'Reply', target ? 'ok' : '');
