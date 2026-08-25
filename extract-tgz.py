import sys, os, tarfile, shutil
cache, target = sys.argv[1], sys.argv[2]
os.makedirs(target, exist_ok=True)
with tarfile.open(cache) as tf:
    names = tf.getnames()
    # find the top-level "package/" dir
    package_dir = None
    for n in names:
        parts = n.split("/")
        if len(parts) >= 2 and parts[0] == "package":
            package_dir = "package"
            break
    if not package_dir:
        package_dir = names[0].split("/")[0]
    # Path traversal protection
    target_real = os.path.realpath(target)
    for m in tf.getmembers():
        dest = os.path.realpath(os.path.join(target, m.name))
        if not dest.startswith(target_real + os.sep) and dest != target_real:
            raise Exception(f"Refusing to extract {m.name}: path traversal detected")
    tf.extractall(target)
src = os.path.join(target, package_dir)
if os.path.isdir(src):
    for name in os.listdir(src):
        shutil.move(os.path.join(src, name), os.path.join(target, name))
    shutil.rmtree(src)
