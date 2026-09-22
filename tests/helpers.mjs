import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const read = path => readFileSync(new URL('../' + path, import.meta.url), 'utf8');
const rect = {x: -800, y: 20, width: 800, height: 600};
const visible = {
    minimized: false, located_on_workspace: () => true,
    showing_on_its_workspace: () => true, get_client_content_rect: () => rect,
    get_title: () => 'Editor', has_focus: () => true, get_pid: () => 42,
    get_window_type: () => 0, get_id: () => 7,
};
const source = read('helpers/gnome/extension.js').replace(/^import .*;$/gm, '')
    .replace('export default class Screenpeek', 'globalThis.Screenpeek = class Screenpeek');
let activated;
const context = {Extension: class {}, Main: {activateWindow: window => { activated = window; }}, Meta: {WindowType: {NORMAL: 0, DIALOG: 4, DESKTOP: 1}}, global: {
    workspace_manager: {get_active_workspace: () => ({})},
    get_window_actors: () => [visible, {...visible, minimized: true},
        {...visible, located_on_workspace: () => false},
        {...visible, get_window_type: () => 1}].map(meta_window => ({meta_window})),
}};
vm.runInNewContext(source, context);
let windows = JSON.parse(new context.Screenpeek().List());
assert.equal(windows.length, 1);
assert.deepEqual(windows[0].at, [-800, 20]);
assert.equal(windows[0].pid, 42);
assert.equal(windows[0].address, '7');
assert.equal(new context.Screenpeek().Focus('7'), true);
assert.equal(activated, visible);
assert.equal(new context.Screenpeek().Focus('8'), false);
visible.get_client_content_rect = undefined;
visible.get_frame_rect = () => rect;
visible.frame_rect_to_client_rect = frame => ({...frame, y: frame.y + 24});
windows = JSON.parse(new context.Screenpeek().List());
assert.deepEqual(windows[0].at, [-800, 44]);

const active = {minimized: false, deleted: false, desktops: [1], activities: [], internalId: {toString: () => '{abc}'},
    caption: 'Editor', clientGeometry: rect, pid: 42};
let callback;
vm.runInNewContext(read('helpers/kwin/windows.js'), {
    SCREENPEEK_CALLBACK: ':1.123',
    workspace: {currentDesktop: 1, currentActivity: 'work', activeWindow: active,
        windowList: () => [active, {...active, desktops: [2]}, {...active, minimized: true},
            {...active, activities: ['other']}, {...active, specialWindow: true}]},
    callDBus: (...args) => { callback = args; },
});
assert.deepEqual(callback.slice(0, 4), [':1.123', '/org/screenpeek/Windows', 'org.screenpeek.Windows', 'Reply']);
windows = JSON.parse(callback[4]);
assert.equal(windows.length, 1);
assert.deepEqual(windows[0].at, [-800, 20]);
assert.equal(windows[0].focusHistoryID, 0);

let focusedReply;
const focusContext = {
    SCREENPEEK_CALLBACK: ':1.9', SCREENPEEK_TARGET: '{abc}',
    workspace: {activeWindow: null, windowList: () => [active]},
    callDBus: (...args) => { focusedReply = args[4]; },
};
vm.runInNewContext(read('helpers/kwin/focus.js'), focusContext);
assert.equal(focusContext.workspace.activeWindow, active);
assert.equal(focusedReply, 'ok');
