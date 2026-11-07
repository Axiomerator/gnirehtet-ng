// Copyright 2026 Gnirehtet NG contributors
// SPDX-License-Identifier: Apache-2.0

package com.genymobile.gnirehtet;

import android.Manifest;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;

import java.util.UUID;

/** Carries the latest phone action over ADB independently of the VPN service. */
public class SessionRequests extends ContentProvider {
    static final String START = "start";
    static final String STOP = "stop";
    static final String RESTART = "restart";

    private static final String KEY_REQUEST = "request";

    private static SharedPreferences preferences(Context context) {
        return context.getSharedPreferences("session_requests", Context.MODE_PRIVATE);
    }

    static synchronized void send(Context context, String action) {
        String request = action + ":" + UUID.randomUUID();
        if (!preferences(context).edit().putString(KEY_REQUEST, request).commit()) {
            Log.w("SessionRequests", "Could not persist phone session request");
        }
    }

    private static synchronized void clear(Context context) {
        preferences(context).edit().remove(KEY_REQUEST).commit();
    }

    private static synchronized void acknowledge(Context context, String request) {
        if (request != null && request.equals(preferences(context).getString(KEY_REQUEST, null))) {
            clear(context);
        }
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Bundle call(String method, String argument, Bundle extras) {
        Context context = getContext();
        context.enforceCallingPermission(Manifest.permission.DUMP, "Session requests require authorized ADB");
        Bundle result = new Bundle();
        switch (method) {
            case "peek":
                result.putString("request", preferences(context).getString(KEY_REQUEST, "none"));
                break;
            case "ack":
                acknowledge(context, argument);
                break;
            case "cancel":
                clear(context);
                break;
            default:
                throw new IllegalArgumentException("Unknown session request method");
        }
        return result;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        throw new UnsupportedOperationException();
    }

    @Override
    public String getType(Uri uri) {
        return null;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException();
    }
}
