/* LOAD/SAVE use fs paths. Serialize/check capacity before truncating a file;
 * LOAD validates the size before replacing the in-memory program. */
static fnd_status tb_file_status;
static void exec_disk(int32_t cmd, Node *path_node, Node *unused) {
  (void)unused;
  fnd_handle file;
  fnd_status status;
  uint32_t size = 0, got = 0;
  tb_file_status = FND_E_BAD_ARG;
  if (!path_node || !*(const char *)path_node) {
    print_string("LOAD/SAVE NEEDS A PATH");
    newline();
    return;
  }
  const char *path = (const char *)path_node;
  if (cmd == SYS_SAVE) {
    disk_limit = sizeof tb_memory->disk_data;
    save_serialize();
    if (error) {
      tb_file_status = FND_E_LIMIT;
      print_string("SAVE BUFFER FULL");
      newline();
      return;
    }
    status = fnd_file_open(path, FND_OPEN_WRITE | FND_OPEN_CREATE | FND_OPEN_TRUNC,
                           FND_MODE_DEFAULT, &file);
    if (status == FND_OK) {
      status = fnd_write(file, tb_memory->disk_data, disk_pos, 0);
      if (status == FND_OK)
        status = fnd_sync(file);
      fnd_status closed = fnd_close(file);
      if (status == FND_OK)
        status = closed;
    }
  } else {
    status = fnd_file_open(path, FND_OPEN_READ, 0, &file);
    if (status == FND_OK) {
      status = fnd_read(file, tb_memory->disk_data, sizeof tb_memory->disk_data, &size);
      if (status == FND_OK && size == sizeof tb_memory->disk_data) {
        char extra;
        status = fnd_read(file, &extra, 1, &got);
        if (status == FND_OK && got)
          status = FND_E_LIMIT;
      }
      fnd_status closed = fnd_close(file);
      if (status == FND_OK)
        status = closed;
      if (status == FND_OK) {
        load_disk_lines(size);
        if (pc >= 0)
          stop = 1;
      }
    }
  }
  tb_file_status = status;
  if (status == FND_OK) {
    if (!tb_batch)
      print_string(cmd == SYS_SAVE ? "SAVED" : "LOADED");
    else
      return;
  } else {
    tb_had_error = 1;
    print_string("FILE ERROR: ");
    print_string(fnd_strerror(status));
  }
  newline();
}
