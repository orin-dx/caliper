/** Forward slashes, so one comparison serves `/` and `C:\` paths. */
export const toPosix = (path: string): string => path.replaceAll('\\', '/')

/** A POSIX root, a Windows drive (`C:/`) or a UNC (`//host`) path. */
export const isAbsolute = (path: string): boolean => /^([a-zA-Z]:)?\//.test(path)
