/*
 * Copyright (C) 2017 Genymobile
 * Modified for Gnirehtet NG by the Gnirehtet NG contributors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package com.genymobile.gnirehtet;

import android.content.Context;
import android.content.SharedPreferences;

/**
 * Persisted Gnirehtet NG client settings. Kept independent of the VPN service so the
 * dashboard activity can read and write them without coupling to the network lifecycle.
 */
public final class Settings {

    private static final String PREFS_NAME = "gnirehtet_ng_settings";
    private static final String KEY_AUTO_RECONNECT = "auto_reconnect";
    private static final String KEY_NOTIFICATION_PERMISSION_REQUESTED = "notification_permission_requested";

    private Settings() {
        // utility class
    }

    private static SharedPreferences preferences(Context context) {
        return context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
    }

    public static boolean isAutoReconnectEnabled(Context context) {
        return preferences(context).getBoolean(KEY_AUTO_RECONNECT, true);
    }

    public static void setAutoReconnectEnabled(Context context, boolean enabled) {
        preferences(context).edit().putBoolean(KEY_AUTO_RECONNECT, enabled).apply();
    }

    public static boolean isNotificationPermissionRequested(Context context) {
        return preferences(context).getBoolean(KEY_NOTIFICATION_PERMISSION_REQUESTED, false);
    }

    public static void setNotificationPermissionRequested(Context context, boolean requested) {
        preferences(context).edit().putBoolean(KEY_NOTIFICATION_PERMISSION_REQUESTED, requested).apply();
    }
}
