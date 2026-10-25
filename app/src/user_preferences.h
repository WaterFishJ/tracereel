#pragma once

#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QSettings>
#include <QtCore/QStandardPaths>
#include <QtCore/QDebug>
#include <windows.h>
#include <string>

static bool trValidTheme(const QString &name) {
    return name == "light" || name == "dark" || name == "system";
}

// Package family identity stays stable across Store updates; package full names
// contain a version and must never be used to locate persistent preferences.
static QString trCurrentPackageFamily() {
    using GetFamily = LONG (WINAPI *)(UINT32 *, PWSTR);
    auto getFamily = reinterpret_cast<GetFamily>(
        GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "GetCurrentPackageFamilyName"));
    if (!getFamily) return {};
    UINT32 length = 0;
    if (getFamily(&length, nullptr) != ERROR_INSUFFICIENT_BUFFER || !length) return {};
    std::wstring family(length, L'\0');
    if (getFamily(&length, family.data()) != ERROR_SUCCESS) return {};
    return QString::fromWCharArray(family.c_str());
}

static QString trPreferencesPathFor(const QString &localData, const QString &family) {
    const QString relative = family.isEmpty()
        ? QStringLiteral("TraceReel/settings.ini")
        : QStringLiteral("Packages/") + family + QStringLiteral("/LocalState/settings.ini");
    return QDir(localData).filePath(relative);
}

static QString trPreferencesFilePath() {
    QString localData = qEnvironmentVariable("LOCALAPPDATA");
    if (localData.isEmpty())
        localData = QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation);
    // Do not fall back to the working directory or installation directory.
    if (localData.isEmpty()) return {};
    return trPreferencesPathFor(localData, trCurrentPackageFamily());
}

static bool trWriteThemePreference(const QString &path, const QString &name) {
    if (path.isEmpty() || !trValidTheme(name)) return false;
    if (!QDir().mkpath(QFileInfo(path).absolutePath())) return false;
    QSettings settings(path, QSettings::IniFormat);
    settings.setAtomicSyncRequired(true);
    settings.setValue("appearance/theme", name);
    settings.sync();
    return settings.status() == QSettings::NoError;
}

static QString trReadThemePreference(const QString &path, const QString &legacyTheme) {
    if (path.isEmpty()) return QStringLiteral("dark");
    if (!QFileInfo::exists(path)) {
        const QString initial = trValidTheme(legacyTheme) ? legacyTheme : QStringLiteral("dark");
        if (!trWriteThemePreference(path, initial))
            qWarning() << "Cannot initialize TraceReel preferences:" << path;
        return initial;
    }
    QSettings settings(path, QSettings::IniFormat);
    const QString name = settings.value("appearance/theme", "dark").toString();
    return trValidTheme(name) ? name : QStringLiteral("dark");
}

static QString trLoadThemePreference() {
    const QString path = trPreferencesFilePath();
    QString legacyTheme;
    // Read the previous registry setting only when creating the new file. An
    // existing INI is authoritative, including one with a missing/invalid key.
    if (!path.isEmpty() && !QFileInfo::exists(path)) {
        QSettings legacy(QSettings::NativeFormat, QSettings::UserScope,
                         "TraceReel", "TraceReel");
        legacyTheme = legacy.value("appearance/theme").toString();
    }
    return trReadThemePreference(path, legacyTheme);
}
