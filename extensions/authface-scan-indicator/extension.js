import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

const STATUS_FILENAME = 'face-auth-status';
const RESULT_SHOW_MS = 1800;
const STALE_SCAN_MS = 10000;

// Session modes in which the unlock UI is on screen. GNOME has no mode called
// 'lock' — the shield is 'lock-screen' and the password/unlock prompt is
// 'unlock-dialog'. Testing for 'lock' matched nothing, so the indicator never
// appeared.
const LOCKED_MODES = ['unlock-dialog', 'lock-screen'];

export default class AuthFaceScanIndicator extends Extension {
    enable() {
        this._monitor = null;
        this._monitorChangedId = null;
        this._hideTimeoutId = null;
        this._staleTimeoutId = null;
        this._pulseStop = false;
        this._scanningShown = false;
        this._resultShown = false;
        this._scanStart = 0;

        this._icon = new St.Icon({
            icon_name: 'camera-photo-symbolic',
            icon_size: 24,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._label = new St.Label({
            text: '',
            y_align: Clutter.ActorAlign.CENTER,
        });

        this._box = new St.BoxLayout({
            reactive: false,
            style: 'spacing: 10px;',
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._box.add_child(this._icon);
        this._box.add_child(this._label);

        this._actor = new St.Bin({
            reactive: false,
            visible: false,
            style: `
                background-color: rgba(0, 0, 0, 0.66);
                border-radius: 20px;
                border: 1px solid rgba(255, 255, 255, 0.18);
                padding: 10px 22px;
            `,
        });
        this._actor.set_child(this._box);

        this._label.set_style('font-size: 16px; font-weight: 600; color: #ffffff;');
        this._icon.set_style('color: #ffffff;');

        Main.uiGroup.add_child(this._actor);

        // Re-centre whenever the bubble's own size changes (the text length
        // differs between states) or the monitor layout changes.
        this._notifyId = this._actor.connect('notify::size', () => this._place());
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => this._place());
        this._modeId = Main.sessionMode.connect('updated', () => this._onModeChanged());

        this._place();
        this._onModeChanged();
    }

    disable() {
        this._stopWatching();
        this._clearHideTimeout();
        this._clearStaleTimeout();
        this._stopPulse();

        if (this._modeId) {
            Main.sessionMode.disconnect(this._modeId);
            this._modeId = null;
        }
        if (this._monitorsId) {
            Main.layoutManager.disconnect(this._monitorsId);
            this._monitorsId = null;
        }
        if (this._actor) {
            if (this._notifyId) {
                this._actor.disconnect(this._notifyId);
                this._notifyId = null;
            }
            Main.uiGroup.remove_child(this._actor);
            this._actor.destroy();
            this._actor = null;
        }
        this._icon = null;
        this._label = null;
        this._box = null;
    }

    _isLocked() {
        return Main.sessionMode && LOCKED_MODES.includes(Main.sessionMode.currentMode);
    }

    _statusPath() {
        return GLib.build_filenamev([GLib.get_user_runtime_dir(), STATUS_FILENAME]);
    }

    /// Watch the status file for changes instead of polling it ten times a
    /// second for the whole session.
    _startWatching() {
        if (this._monitor)
            return;
        try {
            const file = Gio.File.new_for_path(this._statusPath());
            this._monitor = file.monitor_file(Gio.FileMonitorFlags.NONE, null);
            this._monitorChangedId = this._monitor.connect('changed', () => this._onStatusChanged());
        } catch (e) {
            logError(e, 'authFace: could not watch scan status file');
            this._monitor = null;
            return;
        }
        // The helper may have written before the watch was established.
        this._onStatusChanged();
    }

    _stopWatching() {
        if (this._monitorChangedId && this._monitor) {
            this._monitor.disconnect(this._monitorChangedId);
            this._monitorChangedId = null;
        }
        if (this._monitor) {
            this._monitor.cancel();
            this._monitor = null;
        }
    }

    _readStatus() {
        try {
            const [ok, contents] = GLib.file_get_contents(this._statusPath());
            if (!ok || contents === null || contents.length === 0)
                return null;
            return new TextDecoder().decode(contents).trim();
        } catch (e) {
            return null;
        }
    }

    _unlinkStatus() {
        try {
            Gio.File.new_for_path(this._statusPath()).delete(null);
        } catch (e) {
            /* already gone */
        }
    }

    _onModeChanged() {
        if (this._isLocked()) {
            this._startWatching();
        } else {
            this._stopWatching();
            this._scanStart = 0;
            this._hide();
            this._unlinkStatus();
        }
    }

    _onStatusChanged() {
        if (!this._isLocked()) {
            this._hide();
            return;
        }

        const status = this._readStatus();
        if (status === null) {
            this._hide();
            return;
        }

        if (status === 'scanning') {
            if (this._scanStart === 0)
                this._scanStart = GLib.get_monotonic_time() / 1000;
            this._showScanning();
            this._armStaleTimeout();
        } else if (status === 'ok' || status === 'fail') {
            this._scanStart = 0;
            this._clearStaleTimeout();
            this._showResult(status === 'ok');
        }
    }

    /// A crashed helper leaves 'scanning' behind with no further file events,
    /// so the bubble is cleared on a timer rather than on the next poll.
    _armStaleTimeout() {
        this._clearStaleTimeout();
        this._staleTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, STALE_SCAN_MS, () => {
            this._staleTimeoutId = null;
            if (this._readStatus() === 'scanning') {
                this._scanStart = 0;
                this._unlinkStatus();
                this._hide();
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _showScanning() {
        if (this._resultShown || this._scanningShown)
            return;
        this._scanningShown = true;
        this._resultShown = false;

        this._label.set_text('Scanning face…');
        this._icon.icon_name = 'camera-photo-symbolic';
        this._icon.set_style('color: #4dd0e1;');
        this._actor.visible = true;
        this._place();
        this._startPulse();
    }

    _showResult(success) {
        this._scanningShown = false;
        this._stopPulse();
        if (this._resultShown)
            return;
        this._resultShown = true;

        this._label.set_text(
            success ? 'Face recognised' : 'Face not recognised — use your password'
        );
        this._icon.icon_name = success ? 'object-select-symbolic' : 'dialog-error-symbolic';
        this._icon.set_style(success ? 'color: #81c784;' : 'color: #e57373;');
        this._actor.visible = true;
        this._place();

        this._clearHideTimeout();
        this._hideTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RESULT_SHOW_MS, () => {
            this._hideTimeoutId = null;
            this._hide();
            this._unlinkStatus();
            return GLib.SOURCE_REMOVE;
        });
    }

    _startPulse() {
        this._pulseStop = false;
        const tick = visible => {
            if (this._pulseStop || !this._actor)
                return;
            this._actor.ease_property('opacity', visible ? 255 : 120, {
                duration: 450,
                mode: Clutter.AnimationMode.EASE_IN_OUT_SINE,
                onComplete: () => tick(!visible),
            });
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
        if (this._actor)
            this._actor.visible = false;
    }

    _clearHideTimeout() {
        if (this._hideTimeoutId !== null) {
            GLib.source_remove(this._hideTimeoutId);
            this._hideTimeoutId = null;
        }
    }

    _clearStaleTimeout() {
        if (this._staleTimeoutId !== null) {
            GLib.source_remove(this._staleTimeoutId);
            this._staleTimeoutId = null;
        }
    }

    /// Centre the bubble near the top of the primary monitor. The previous
    /// version set only `y`, leaving it pinned to the left edge.
    _place() {
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor || !this._actor)
            return;
        const width = this._actor.width;
        this._actor.x = monitor.x + Math.max(0, Math.floor((monitor.width - width) / 2));
        this._actor.y = monitor.y + 24;
    }
}
