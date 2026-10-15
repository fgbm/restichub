#!/usr/bin/env python3
"""Write an agent package whose bytes depend only on the file contents (NFR-SEC-06).

Entries are sorted by name and carry a fixed owner, group, mode and modification time. The
gzip header carries no name and no time.
"""
import argparse
import gzip
import io
import os
import tarfile
import zipfile


def mode_of(name):
    return 0o755 if name.startswith("restichub-agent") else 0o644


def write_tar(out, directory, names, epoch):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as tar:
        for name in names:
            path = os.path.join(directory, name)
            info = tarfile.TarInfo(name)
            info.size = os.path.getsize(path)
            info.mtime = epoch
            info.mode = mode_of(name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            with open(path, "rb") as handle:
                tar.addfile(info, handle)
    with open(out, "wb") as handle:
        with gzip.GzipFile(filename="", mode="wb", fileobj=handle, mtime=0, compresslevel=9) as gz:
            gz.write(raw.getvalue())


def write_zip(out, directory, names, epoch):
    # ZIP stores local time without a zone, and cannot hold a date before 1980.
    stamp = max(epoch, 315532800)
    date_time = tuple(__import__("time").gmtime(stamp))[:6]
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name in names:
            info = zipfile.ZipInfo(name, date_time)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o100000 | mode_of(name)) << 16
            info.create_system = 3
            with open(os.path.join(directory, name), "rb") as handle:
                archive.writestr(info, handle.read(), compresslevel=9)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--epoch", type=int, required=True)
    parser.add_argument("names", nargs="+")
    args = parser.parse_args()
    names = sorted(args.names)
    if args.out.endswith(".tar.gz"):
        write_tar(args.out, args.directory, names, args.epoch)
    elif args.out.endswith(".zip"):
        write_zip(args.out, args.directory, names, args.epoch)
    else:
        parser.error("the output must end in .tar.gz or .zip")


if __name__ == "__main__":
    main()
