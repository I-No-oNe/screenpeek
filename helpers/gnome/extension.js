import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const SERVICE = 'org.screenpeek.Windows';
const PATH = '/org/screenpeek/Windows';
const XML = `<node><interface name="${SERVICE}">
    <method name="List"><arg type="s" direction="out"/></method>
    <method name="Focus"><arg type="s" direction="in"/><arg type="b" direction="out"/></method>
</interface></node>`;

export default class Screenpeek extends Extension {
    enable() {
        this._object = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._object.export(Gio.DBus.session, PATH);
        this._owner = Gio.bus_own_name_on_connection(
            Gio.DBus.session, SERVICE, Gio.BusNameOwnerFlags.NONE, null, null);
    }

    List() {
        const workspace = global.workspace_manager.get_active_workspace();
        return JSON.stringify(global.get_window_actors()
            .map(actor => actor.meta_window)
            .filter(window => !window.minimized && window.located_on_workspace(workspace)
                && [Meta.WindowType.NORMAL, Meta.WindowType.DIALOG].includes(window.get_window_type())
                && window.showing_on_its_workspace())
            .map(window => {
                // Newer Mutter exposes the exact AT-SPI origin; older versions use the client rect.
                const rect = window.get_client_content_rect?.()
                    ?? window.frame_rect_to_client_rect(window.get_frame_rect());
                return {
                    title: window.get_title() ?? '',
                    at: [Math.round(rect.x), Math.round(rect.y)],
                    size: [Math.round(rect.width), Math.round(rect.height)],
                    mapped: true,
                    focusHistoryID: window.has_focus() ? 0 : 1,
                    pid: window.get_pid() || null,
                    address: String(window.get_id()),
                };
            }));
    }

    Focus(id) {
        const window = global.get_window_actors()
            .map(actor => actor.meta_window)
            .find(window => String(window.get_id()) === id);
        if (!window) return false;
        Main.activateWindow(window);
        return true;
    }

    disable() {
        this._object?.unexport();
        this._object = null;
        if (this._owner) Gio.bus_unown_name(this._owner);
        this._owner = 0;
    }
}
