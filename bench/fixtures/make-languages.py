#!/usr/bin/env python3
"""Generate language fixtures with Pango shaping; requires pango-view and magick."""
from pathlib import Path
import json
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
LABELS = {
    "eng": ("Liberation Sans", "eng", ["Preferences", "Save", "Cancel", "Open file", "Font size", "Save 3 files",
                                       "Edit", "Copy", "Paste", "Find and replace", "New window", "Zoom 100%"]),
    "fra": ("Liberation Sans", "fra", ["Préférences", "Enregistrer", "Annuler", "Ouvrir un fichier", "Taille du texte", "Enregistrer 3 fichiers",
                                       "Modifier", "Copier", "Coller", "Rechercher et remplacer", "Nouvelle fenêtre", "Zoom 100%"]),
    "deu": ("Liberation Sans", "deu", ["Einstellungen", "Speichern", "Abbrechen", "Datei öffnen", "Schriftgröße", "3 Dateien speichern",
                                       "Bearbeiten", "Kopieren", "Einfügen", "Suchen und ersetzen", "Neues Fenster", "Zoom 100%"]),
    "heb": ("Noto Sans Hebrew", "heb", ["הגדרות", "שמירה", "ביטול", "פתיחת קובץ", "גודל גופן", "שמור 3 קבצים",
                                        "עריכה", "העתקה", "הדבקה", "חיפוש והחלפה", "חלון חדש", "סגירה"]),
    "ara": ("Noto Naskh Arabic", "eng+ara", ["الإعدادات", "حفظ", "إلغاء", "فتح ملف", "حجم الخط", "حفظ PDF",
                                             "تحرير", "نسخ", "لصق", "بحث واستبدال", "نافذة جديدة", "إغلاق"]),
    "jpn": ("Noto Sans CJK JP", "jpn", ["設定", "保存", "キャンセル", "ファイルを開く", "文字サイズ", "PDFを保存",
                                        "編集", "コピー", "貼り付け", "検索と置換", "新しいウィンドウ", "閉じる"]),
    "chi_sim": ("Noto Sans CJK SC", "chi_sim", ["设置", "保存", "取消", "打开文件", "字体大小", "保存PDF",
                                                "编辑", "复制", "粘贴", "查找和替换", "新建窗口", "关闭"]),
    "rus": ("Liberation Sans", "rus", ["Настройки", "Сохранить", "Отмена", "Открыть файл", "Размер шрифта", "Сохранить 3 файла",
                                       "Правка", "Копировать", "Вставить", "Найти и заменить", "Новое окно", "Закрыть"]),
}


def main():
    with tempfile.TemporaryDirectory(prefix="screenpeek-labels-") as tmp:
        for lang, (font, language, labels) in LABELS.items():
            matched = subprocess.run(["fc-match", "--format=%{family}", font],
                                     check=True, capture_output=True, text=True).stdout
            if font not in matched.split(","):
                raise SystemExit(f"install font {font!r}; fontconfig substituted {matched!r}")
            targets = []
            command = ["magick", "-size", "640x780", "xc:white"]
            for i, label in enumerate(labels):
                tile = str(Path(tmp) / f"{lang}-{i}.png")
                subprocess.run(["pango-view", "--no-display", "--pixels", "--margin=0",
                                f"--font={font} 18", f"--text={label}", f"--output={tile}"], check=True)
                width, height = struct.unpack(">II", Path(tile).read_bytes()[16:24])
                targets.append(dict(label=label, x=30, y=30 + i * 60, width=width, height=height))
                command += [tile, "-geometry", f"+30+{30 + i * 60}", "-composite"]
            subprocess.run(command + [str(ROOT / f"{lang}.png")], check=True)
            (ROOT / f"{lang}.json").write_text(json.dumps(dict(image=f"{lang}.png", language=language, targets=targets), ensure_ascii=False, indent=2) + "\n")
            (ROOT / f"{lang}.expected").write_text("\n".join(labels) + "\n")


if __name__ == "__main__":
    main()
