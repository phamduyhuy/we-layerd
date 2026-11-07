const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const {test} = require('node:test');
const source = readFileSync(`${__dirname}/../we-layerd@aromatic/extension.js`, 'utf8')
    .replace(/^import .*;\n/gm, '')
    .replace('export default class WeLayerdExtension', 'class WeLayerdExtension');
function setup({renderer = true, title, wmClass, active = true, visible = true, sticky = false} = {}) {
    const wallpaper = {
        get_pid: () => 123,
        get_title: () => title ?? 'we-layerd-renderer-123',
        get_wm_class: () => wmClass ?? 'io.github.weLayerd.Renderer',
        get_window_type: () => 0,
        showing_on_its_workspace: () => true,
    };
    const app = {
        get_pid: () => 456,
        get_title: () => 'Browser',
        get_wm_class: () => 'browser',
        get_window_type: () => 0,
        showing_on_its_workspace: () => visible,
        located_on_workspace: workspace => workspace === 'active' && (active || sticky),
        get_monitor: () => 0,
        maximized_horizontally: true,
        maximized_vertically: true,
    };
    class Component {
        constructor() { this.actors = 0; }
        enable() {} disable() { this.actors = 0; } refreshMatches() {}
        reloadBackgrounds() { this.actors++; }
    }
    const context = vm.createContext({
        console: {log() {}}, TextDecoder, Extension: class {},
        GnomeShellOverride: Component, WindowManager: Component,
        Meta: {WindowType: {NORMAL: 0}},
        global: {
            workspace_manager: {get_active_workspace: () => 'active'},
            display: {list_all_windows: () => renderer ? [wallpaper, app] : [app], get_focus_window: () => app},
            backend: {get_monitor_manager: () => ({get_logical_monitors: () => [{
                get_number: () => 0,
                get_monitors: () => [{get_connector: () => 'eDP-1'}],
            }]})},
        },
        Gio: {
            DBusExportedObject: {wrapJSObject: () => ({export() {}, unexport() {}})},
            BusType: {SESSION: 0}, BusNameOwnerFlags: {REPLACE: 0},
            bus_get_sync() {}, bus_own_name_on_connection: () => 1, bus_unown_name() {},
        },
    });
    const extension = vm.runInContext(source + '\nnew WeLayerdExtension()', context);
    extension.dir = {get_child: () => ({load_contents: () => [true, new Uint8Array()]})};
    return {extension, wallpaper};
}
test('re-enabling extension recovers live wallpaper registration and pause rules', () => {
    const {extension, wallpaper} = setup();
    extension.enable();
    extension.RegisterWindow(123, 123, 'we-layerd-renderer-123', 'io.github.weLayerd.Renderer');
    extension.disable();
    extension.enable();
    assert.equal(extension._matches(wallpaper), true, 'wallpaper must be managed again');
    assert.equal(extension._override.actors, 1, 'wallpaper background must be rebuilt');
    assert.deepEqual(JSON.parse(JSON.stringify(extension.GetWindowStates())), [['eDP-1', true, true, false]]);
});

test('cold start without renderer waits for normal registration', () => {
    const {extension} = setup({renderer: false});
    extension.enable();
    assert.equal(extension._target, null);
    extension.RegisterWindow(123, 123, 'we-layerd-renderer-123', 'io.github.weLayerd.Renderer');
    assert.equal(extension._target.pid, 123);
});
test('recovery requires both renderer app ID and PID-specific title', () => {
    for (const identity of [{title: 'we-layerd-renderer-999'}, {wmClass: 'browser'}]) {
        const {extension} = setup(identity);
        extension.enable();
        assert.equal(extension._target, null);
    }
});

test('rules exclude inactive workspace and minimized windows but include sticky windows', () => {
    for (const options of [{active: false}, {visible: false}]) {
        const {extension} = setup(options);
        extension.enable();
        assert.equal(extension.GetWindowStates().length, 0);
    }
    const {extension} = setup({active: false, sticky: true});
    extension.enable();
    assert.equal(extension.GetWindowStates().length, 1);
});
