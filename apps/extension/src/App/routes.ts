export default {
  approveTx: (): string => `/tx`,
  login: (): string => `/login`,
  approveConnection: (): string => `/connection`,
  setup: (): string => `/setup`,
  changePassword: (): string => `/change-password`,
  connectedSites: (): string => `/connected-sites`,

  viewAccountList: () => `/accounts/view`,
  viewAccountMnemonic: (accountId: string = ":accountId") =>
    `/accounts/mnemonic/${accountId}`,
  viewAccount: (accountId: string = ":accountId") =>
    `/accounts/view/${accountId}`,
  deleteAccount: (accountId: string = ":accountId") =>
    `/accounts/delete/${accountId}`,
  renameAccount: (accountId: string = ":accountId") =>
    `/accounts/rename/${accountId}`,
};
