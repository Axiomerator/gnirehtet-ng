package com.genymobile.gnirehtet;

import android.Manifest;
import android.annotation.TargetApi;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.Build;

/**
 * Manage the notification necessary for the foreground service (mandatory since Android O).
 */
public class Notifier {

    private static final int NOTIFICATION_ID = 42;
    private static final String CHANNEL_ID = "gnirehtet_ng_vpn";

    private final Service context;
    private boolean failure;
    private boolean foreground;

    public Notifier(Service context) {
        this.context = context;
    }

    private Notification createNotification(boolean failed) {
        Notification.Builder builder = createNotificationBuilder()
                .setContentTitle(context.getString(R.string.app_name))
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setCategory(Notification.CATEGORY_SERVICE)
                .setContentIntent(createDashboardPendingIntent());
        if (failed) {
            builder.setContentText(context.getString(R.string.relay_disconnected));
        } else {
            builder.setContentText(context.getString(R.string.relay_connected));
        }
        builder.setSmallIcon(R.drawable.ic_stat_gnirehtet);
        builder.addAction(createStopAction());
        return builder.build();
    }

    private PendingIntent createDashboardPendingIntent() {
        Intent intent = new Intent(context, MainActivity.class);
        int flags = PendingIntent.FLAG_ONE_SHOT;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            flags |= PendingIntent.FLAG_IMMUTABLE;
        }
        return PendingIntent.getActivity(context, 1, intent, flags);
    }

    @SuppressWarnings("deprecation")
    private Notification.Builder createNotificationBuilder() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            return new Notification.Builder(context, CHANNEL_ID);
        }
        return new Notification.Builder(context);
    }

    @TargetApi(Build.VERSION_CODES.O)
    private void createNotificationChannel() {
        NotificationChannel channel = new NotificationChannel(CHANNEL_ID,
                context.getString(R.string.app_name), NotificationManager.IMPORTANCE_LOW);
        channel.setDescription(context.getString(R.string.notification_channel_description));
        channel.setShowBadge(false);
        getNotificationManager().createNotificationChannel(channel);
    }

    public void start() {
        failure = false;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            createNotificationChannel();
        }
        Notification notification = createNotification(false);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            context.startForeground(NOTIFICATION_ID, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            context.startForeground(NOTIFICATION_ID, notification);
        }
        foreground = true;
    }

    public void stop() {
        if (!foreground) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            context.stopForeground(Service.STOP_FOREGROUND_REMOVE);
        } else {
            context.stopForeground(true);
        }
        foreground = false;
    }

    public void setFailure(boolean failed) {
        if (failure != failed) {
            failure = failed;
            if (canPostNotifications()) {
                getNotificationManager().notify(NOTIFICATION_ID, createNotification(failed));
            }
        }
    }

    private boolean canPostNotifications() {
        return Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU
                || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        == PackageManager.PERMISSION_GRANTED;
    }

    private Notification.Action createStopAction() {
        Intent stopIntent = new Intent(context, MainActivity.class)
                .setAction(GnirehtetActivity.ACTION_GNIREHTET_STOP);
        int flags = PendingIntent.FLAG_ONE_SHOT;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            flags |= PendingIntent.FLAG_IMMUTABLE;
        }
        PendingIntent stopPendingIntent = PendingIntent.getActivity(context, 0, stopIntent, flags);
        // the non-deprecated constructor is not available in API 21
        @SuppressWarnings("deprecation")
        Notification.Action.Builder actionBuilder = new Notification.Action.Builder(R.drawable.ic_close_24dp, context.getString(R.string.stop_vpn),
                stopPendingIntent);
        return actionBuilder.build();
    }

    private NotificationManager getNotificationManager() {
        return (NotificationManager) context.getSystemService(Context.NOTIFICATION_SERVICE);
    }
}
