import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const read = path => readFileSync(new URL('../' + path, import.meta.url), 'utf8');
const rect = {x: -800, y: 20, width: 800, height: 600};
const visible = {
    minimized: false, located_on_workspace: () => true,
    showing_on_its_workspace: () => true, get_client_content_rect: () => rect,
    get_title: () => 'Editor', has_focus: () => true, get_pid: () => 42,
};
const source = read('helpers/gnome/extension.js').replace(/^import .*;$/gm, '')
    .replace('export default class Screenpeek', 'globalThis.Screenpeek = class Screenpeek');
const context = {Extension: class {}, global: {
    workspace_manager: {get_active_workspace: () => ({})},
    get_window_actors: () => [visible, {...visible, minimized: true},
        {...visible, located_on_workspace: () => false}].map(meta_window => ({meta_window})),
}};
vm.runInNewContext(source, context);
let windows = JSON.parse(new context.Screenpeek().List());
assert.equal(windows.length, 1);
assert.deepEqual(windows[0].at, [-800, 20]);
assert.equal(windows[0].pid, 42);
visible.get_client_content_rect = undefined;
visible.get_frame_rect = () => rect;
visible.frame_rect_to_client_rect = frame => ({...frame, y: frame.y + 24});
windows = JSON.parse(new context.Screenpeek().List());
assert.deepEqual(windows[0].at, [-800, 44]);

const active = {minimized: false, deleted: false, desktops: [1], activities: [],
    caption: 'Editor', clientGeometry: rect, pid: 42};
let callback;
vm.runInNewContext(read('helpers/kwin/windows.js'), {
    SCREENPEEK_CALLBACK: ':1.123',
    workspace: {currentDesktop: 1, currentActivity: 'work', activeWindow: active,
        windowList: () => [active, {...active, desktops: [2]}, {...active, minimized: true},
            {...active, activities: ['other']}]},
    callDBus: (...args) => { callback = args; },
});
assert.deepEqual(callback.slice(0, 4), [':1.123', '/org/screenpeek/Windows', 'org.screenpeek.Windows', 'Reply']);
windows = JSON.parse(callback[4]);
assert.equal(windows.length, 1);
assert.deepEqual(windows[0].at, [-800, 20]);
assert.equal(windows[0].focusHistoryID, 0);
console.log('GNOME and KWin helper contract checks passed (mock compositor objects).');
