package com.flectar.mail;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.IntentSender;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Base64;

import com.google.android.gms.auth.api.identity.AuthorizationRequest;
import com.google.android.gms.auth.api.identity.AuthorizationResult;
import com.google.android.gms.auth.api.identity.Identity;
import com.google.android.gms.common.api.ApiException;
import com.google.android.gms.common.api.Scope;

import java.util.ArrayList;
import java.util.List;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;

/** NativeActivity host plus the supported Google AuthorizationClient bridge. */
public final class FlectarActivity extends NativeActivity {
    private static final int GOOGLE_AUTHORIZATION_REQUEST = 0xF1EC;
    private long pendingGoogleRequest = -1;

    private static native void nativeGoogleAuthorizationResult(
            long requestId,
            String accessToken,
            long expiresInSeconds,
            String error);

    private static native void nativeMicrosoftRedirect(String redirectUri);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (savedInstanceState != null) {
            pendingGoogleRequest = savedInstanceState.getLong("pendingGoogleRequest", -1);
        }
    }

    @Override
    protected void onSaveInstanceState(Bundle state) {
        state.putLong("pendingGoogleRequest", pendingGoogleRequest);
        super.onSaveInstanceState(state);
    }

    /** Entra binds Android redirects to the certificate of the installed APK. */
    public String microsoftRedirectUri() {
        try {
            Signature signature;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                PackageInfo info = getPackageManager().getPackageInfo(
                        getPackageName(), PackageManager.GET_SIGNING_CERTIFICATES);
                Signature[] signers = info.signingInfo.getApkContentsSigners();
                signature = signers[0];
            } else {
                @SuppressWarnings("deprecation")
                PackageInfo info = getPackageManager().getPackageInfo(
                        getPackageName(), PackageManager.GET_SIGNATURES);
                @SuppressWarnings("deprecation")
                Signature legacySignature = info.signatures[0];
                signature = legacySignature;
            }
            byte[] digest = MessageDigest.getInstance("SHA-1").digest(signature.toByteArray());
            String hash = Base64.encodeToString(digest, Base64.NO_WRAP);
            return "msauth://" + getPackageName() + "/" + Uri.encode(hash);
        } catch (PackageManager.NameNotFoundException | NoSuchAlgorithmException
                 | NullPointerException | ArrayIndexOutOfBoundsException error) {
            return null;
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (intent.getData() != null
                && "msauth".equals(intent.getData().getScheme())
                && "com.flectar.mail".equals(intent.getData().getHost())) {
            nativeMicrosoftRedirect(intent.getData().toString());
        }
    }

    /** Called from Rust. Must return quickly; Play Services completes asynchronously. */
    public void authorizeGoogle(String[] requestedScopes, long requestId, boolean interactive) {
        runOnUiThread(() -> {
            List<Scope> scopes = new ArrayList<>(requestedScopes.length);
            for (String scope : requestedScopes) {
                scopes.add(new Scope(scope));
            }
            AuthorizationRequest request = AuthorizationRequest.builder()
                    .setRequestedScopes(scopes)
                    .build();
            Identity.getAuthorizationClient(this)
                    .authorize(request)
                    .addOnSuccessListener(result -> handleAuthorization(result, requestId, interactive))
                    .addOnFailureListener(error -> deliverFailure(requestId, error));
        });
    }

    private void handleAuthorization(
            AuthorizationResult result,
            long requestId,
            boolean interactive) {
        if (!result.hasResolution()) {
            deliverSuccess(requestId, result);
            return;
        }
        if (!interactive) {
            nativeGoogleAuthorizationResult(
                    requestId, "", 0, "needs_reauth:consent or account selection is required");
            return;
        }
        pendingGoogleRequest = requestId;
        try {
            startIntentSenderForResult(
                    result.getPendingIntent().getIntentSender(),
                    GOOGLE_AUTHORIZATION_REQUEST,
                    null,
                    0,
                    0,
                    0);
        } catch (IntentSender.SendIntentException error) {
            pendingGoogleRequest = -1;
            deliverFailure(requestId, error);
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != GOOGLE_AUTHORIZATION_REQUEST) {
            return;
        }
        long requestId = pendingGoogleRequest;
        pendingGoogleRequest = -1;
        if (requestId < 0 || data == null) {
            if (requestId >= 0) {
                nativeGoogleAuthorizationResult(requestId, "", 0, "authorization cancelled");
            }
            return;
        }
        try {
            AuthorizationResult result = Identity.getAuthorizationClient(this)
                    .getAuthorizationResultFromIntent(data);
            deliverSuccess(requestId, result);
        } catch (ApiException error) {
            deliverFailure(requestId, error);
        }
    }

    private static void deliverSuccess(long requestId, AuthorizationResult result) {
        String token = result.getAccessToken();
        if (token == null || token.isEmpty()) {
            nativeGoogleAuthorizationResult(requestId, "", 0, "no access token returned");
        } else {
            // Google access tokens are currently one hour. Rust subtracts a
            // safety window and asks AuthorizationClient for another token.
            nativeGoogleAuthorizationResult(requestId, token, 3600, "");
        }
    }

    private static void deliverFailure(long requestId, Exception error) {
        String detail = error.getMessage();
        nativeGoogleAuthorizationResult(
                requestId,
                "",
                0,
                detail == null || detail.isEmpty() ? error.getClass().getSimpleName() : detail);
    }
}
