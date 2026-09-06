import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

const STATUS_FILENAME = 'face-auth-status';
const POLL_MS = 100;
const RESULT_SHOW_MS = 1800;
const STALE_SCAN_MS = 10000;

export default class AuthFaceScanIndicator extends Extension {
    enable() {
        this._pollId = null;
        this._hideTimeoutId = null;
        this._pulseStop = false;
        this._scanningShown = false;
        this._resultShown = false;
        this._scanStart = 0;

        this._box = new St.BoxLayout({ reactive: false });
        this._icon = new St.Icon({
            icon_name: 'camera-symbolic',
            icon_size: 24,
            reactive: false,
        });
        this._label = new St.Label({
            text: '',
            reactive: false,
            y_align: St.Align.MIDDLE,
        });

        this._box.add_child(this._icon);
        this._box.add_child(this._label);

        this._actor = new St.Bin({
            x_expand: true,
            y_expand: false,
            y_align: St.Align.START,
            reactive: false,
            visible: false,
        });
        this._box.x_align = St.Align.MIDDLE;
        this._box.y_align = St.Align.MIDDLE;
        this._actor.set_child(this._box);

        const style = `
            background-color: rgba(0, 0, 0, 0.66);
            border-radius: 20px;
            border: 1px solid rgba(255, 255, 255, 0.18);
            padding: 10px 22px;
            margin-top: 24px;
        `;
        this._actor.set_style(style);

        const labelStyle = `
            font-size: 16px;
            font-weight: 600;
            color: #ffffff;
        `;
        this._label.set_style(labelStyle);
        this._icon.set_style('color: #ffffff;');

        Main.uiGroup.add_child(this._actor);
        this._place();

        this._widthId = this._actor.connect('notify::width', () => this._place());
        this._monitorId = Main.layoutManager.connect('monitors-changed', () => this._place());
        this._modeId = Main.sessionMode.connect('updated', () => {
            this._onModeChanged();
        });

        this._pollId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, POLL_MS, () => {
            this._poll();
            return GLib.SOURCE_CONTINUE;
        });
    }

    disable() {
        if (this._pollId !== null) {
            GLib.source_remove(this._pollId);
            this._pollId = null;
        }
        this._clearHideTimeout();
        this._stopPulse();

        if (this._actor) {
            if (this._widthId) this._actor.disconnect(this._widthId);
            Main.uiGroup.remove_child(this._actor);
            this._actor.destroy();
            this._actor = null;
        }
        if (this._monitorId) {
            Main.layoutManager.disconnect(this._monitorId);
            this._monitorId = null;
        }
        if (this._modeId) {
            Main.sessionMode.disconnect(this._modeId);
            this._modeId = null;
        }
    }

    _statusPath() {
        return GLib.build_filenamev([GLib.get_user_runtime_dir(), STATUS_FILENAME]);
    }

    _readStatus() {
        try {
            const [, contents] = GLib.file_get_contents(this._statusPath());
            if (contents === null || contents.length === 0) {
                return null;
            }
            return contents.toString().trim();
        } catch (e) {
            return null;
        }
    }

    _unlinkStatus() {
        try {
            const file = Gio.File.new_for_path(this._statusPath());
            file.delete(null);
        } catch (e) {
            /* ignore */
        }
    }

    _onModeChanged() {
        const locked = Main.sessionMode && Main.sessionMode.currentMode === 'lock';
        if (!locked) {
            this._hide();
            return;
        }
        this._scanStart = 0;
        this._hide();
        this._unlinkStatus();
    }

    _poll() {
        const locked = Main.sessionMode && Main.sessionMode.currentMode === 'lock';
        if (!locked) {
            this._hide();
            return;
        }

        const status = this._readStatus();
        if (status === null) {
            this._hide();
            return;
        }

        const now = Date.now();
        if (status === 'scanning') {
            if (this._scanStart === 0) {
                this._scanStart = now;
            }
            if (now - this._scanStart > STALE_SCAN_MS) {
                this._scanStart = 0;
                this._unlinkStatus();
                this._hide();
                return;
            }
            this._showScanning();
        } else if (status === 'ok' || status === 'fail') {
            this._scanStart = 0;
            this._showResult(status === 'ok');
        }
    }

    _showScanning() {
        if (this._resultShown || this._scanningShown) {
            return;
        }
        this._scanningShown = true;
        this._resultShown = false;

        this._label.set_text('Memindai wajah…');
        this._icon.icon_name = 'camera-symbolic';
        this._icon.set_style('color: #4dd0e1;');
        this._actor.visible = true;
        this._place();
        this._startPulse();
    }

    _showResult(success) {
        this._scanningShown = false;
        this._stopPulse();
        if (this._resultShown) {
            return;
        }
        this._resultShown = true;

        this._label.set_text(
            success ? 'Wajah dikenali' : 'Wajah tidak dikenali — gunakan password'
        );
        this._icon.icon_name = success ? 'object-select-symbolic' : 'dialog-error-symbolic';
        this._icon.set_style(success ? 'color: #81c784;' : 'color: #e57373;');
        this._actor.visible = true;
        this._place();

        this._clearHideTimeout();
        this._hideTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RESULT_SHOW_MS, () => {
            this._hide();
            this._unlinkStatus();
            this._hideTimeoutId = null;
            return GLib.SOURCE_REMOVE;
        });
    }

    _startPulse() {
        this._pulseStop = false;
        const tick = (visible) => {
            if (this._pulseStop) {
                return;
            }
            this._actor.ease_property(
                'opacity',
                visible ? 255 : 120,
                {
                    duration: 450,
                    mode: Clutter.AnimationMode.EASE_IN_OUT_SINE,
                    onComplete: () => tick(!visible),
                }
            );
        };
        this._actor.opacity = 255;
        tick(true);
    }

    _stopPulse() {
        this._pulseStop = true;
        if (this._actor) {
            this._actor.remove_all_transitions();
            this._actor.opacity = 255;
        }
    }

    _hide() {
        this._scanningShown = false;
        this._resultShown = false;
        this._clearHideTimeout();
        this._stopPulse();
        if (this._actor) {
            this._actor.visible = false;
        }
    }

    _clearHideTimeout() {
        if (this._hideTimeoutId !== null) {
            GLib.source_remove(this._hideTimeoutId);
            this._hideTimeoutId = null;
        }
    }

    _place() {
        if (!Main.layoutManager.primaryMonitor || !this._actor) {
            return;
        }
        const monitor = Main.layoutManager.primaryMonitor;
        this._actor.y = monitor.y + 24;
    }
}